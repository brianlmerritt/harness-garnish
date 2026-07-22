use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

const MAX_UNTRUSTED_PATCH_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepositorySnapshot {
    pub root: String,
    pub base_commit: String,
    pub branch: Option<String>,
    pub status_porcelain_v2: String,
    pub remotes: String,
    pub submodules: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Worktree {
    pub path: String,
    pub branch: String,
    pub base_commit: String,
}

pub fn create_verification_worktree(
    repository: &Path,
    destination: &Path,
    base_commit: &str,
    patch: &[u8],
) -> Result<Worktree> {
    let before = snapshot(repository)?;
    if destination.exists() {
        bail!(
            "verification worktree destination already exists: {}",
            destination.display()
        );
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    let destination_arg = destination.to_string_lossy().into_owned();
    let output = git_output(
        repository,
        &["worktree", "add", "--detach", &destination_arg, base_commit],
    )?;
    if !output.status.success() {
        bail!(
            "git verification worktree add failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    if !patch.is_empty() {
        let mut child = Command::new("git")
            .args(["apply", "--binary", "-"])
            .current_dir(destination)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("starting git apply for verifier")?;
        child
            .stdin
            .take()
            .context("opening git apply stdin")?
            .write_all(patch)?;
        let applied = child.wait_with_output()?;
        if !applied.status.success() {
            bail!(
                "applying task patch in verification worktree failed: {}",
                String::from_utf8_lossy(&applied.stderr).trim()
            );
        }
    }
    let after = snapshot(repository)?;
    if after.status_porcelain_v2 != before.status_porcelain_v2
        || after.base_commit != before.base_commit
        || after.branch != before.branch
    {
        bail!("source checkout changed while creating verification worktree");
    }
    Ok(Worktree {
        path: destination.canonicalize()?.to_string_lossy().into_owned(),
        branch: "(detached verifier)".into(),
        base_commit: base_commit.into(),
    })
}

pub fn snapshot(repository: &Path) -> Result<RepositorySnapshot> {
    let root = git_text(repository, &["rev-parse", "--show-toplevel"])?;
    let root_path = PathBuf::from(root.trim());
    let base_commit = git_text(&root_path, &["rev-parse", "HEAD"])?;
    let branch = git_output(&root_path, &["symbolic-ref", "--quiet", "--short", "HEAD"])
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .filter(|value| !value.is_empty());
    let status_porcelain_v2 = git_text(
        &root_path,
        &["status", "--porcelain=v2", "--untracked-files=all"],
    )?;
    let remotes = git_text(&root_path, &["remote", "-v"])?;
    let submodules = git_output(&root_path, &["submodule", "status"])
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
        .unwrap_or_default();
    Ok(RepositorySnapshot {
        root: root_path.to_string_lossy().into_owned(),
        base_commit: base_commit.trim().to_owned(),
        branch,
        status_porcelain_v2,
        remotes,
        submodules,
    })
}

pub fn create_task_worktree(
    repository: &Path,
    destination: &Path,
    task_id: &str,
) -> Result<Worktree> {
    let before = snapshot(repository)?;
    if has_user_changes(&before.status_porcelain_v2) {
        bail!(
            "source checkout is dirty; refusing to create an automated write task without an explicit dirty-tree policy"
        );
    }
    if destination.exists() {
        bail!(
            "worktree destination already exists: {}",
            destination.display()
        );
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    let suffix: String = task_id
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .take(16)
        .collect();
    if suffix.is_empty() {
        bail!("task id cannot form a branch name");
    }
    let branch = format!("garnish/task-{suffix}");
    let destination_arg = destination.to_string_lossy().into_owned();
    let output = git_output(
        repository,
        &[
            "worktree",
            "add",
            "-b",
            &branch,
            &destination_arg,
            &before.base_commit,
        ],
    )?;
    if !output.status.success() {
        bail!(
            "git worktree add failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let after = snapshot(repository)?;
    if after.status_porcelain_v2 != before.status_porcelain_v2
        || after.base_commit != before.base_commit
        || after.branch != before.branch
    {
        bail!("source checkout changed while creating task worktree");
    }
    Ok(Worktree {
        path: destination.canonicalize()?.to_string_lossy().into_owned(),
        branch,
        base_commit: before.base_commit,
    })
}

pub fn create_or_reuse_task_worktree(
    repository: &Path,
    destination: &Path,
    task_id: &str,
) -> Result<Worktree> {
    if !destination.exists() {
        return create_task_worktree(repository, destination, task_id);
    }
    let source = snapshot(repository)?;
    if has_user_changes(&source.status_porcelain_v2) {
        bail!("source checkout is dirty; refusing to adopt a prepared task worktree");
    }
    let prepared = snapshot(destination)?;
    let suffix: String = task_id
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .take(16)
        .collect();
    if suffix.is_empty() {
        bail!("task id cannot form a branch name");
    }
    let expected_branch = format!("garnish/task-{suffix}");
    let canonical_destination = destination.canonicalize()?;
    if Path::new(&prepared.root) != canonical_destination {
        bail!("prepared worktree root does not match its expected destination");
    }
    if prepared.branch.as_deref() != Some(expected_branch.as_str()) {
        bail!("prepared worktree branch does not match task {task_id}");
    }
    if prepared.base_commit != source.base_commit {
        bail!("prepared worktree base no longer matches the source checkout");
    }
    if has_user_changes(&prepared.status_porcelain_v2) {
        bail!("prepared worktree contains changes and cannot be adopted before claim consumption");
    }
    Ok(Worktree {
        path: canonical_destination.to_string_lossy().into_owned(),
        branch: expected_branch,
        base_commit: source.base_commit,
    })
}

fn has_user_changes(status: &str) -> bool {
    status.lines().any(|line| {
        let path = line.split_whitespace().last().unwrap_or_default();
        !path.starts_with(".harness-garnish/") && path != ".harness-garnish"
    })
}

pub fn patch(worktree: &Path) -> Result<Vec<u8>> {
    let intent = git_output(worktree, &["add", "--intent-to-add", "--", "."])?;
    if !intent.status.success() {
        bail!(
            "git add --intent-to-add failed: {}",
            String::from_utf8_lossy(&intent.stderr)
        );
    }
    let output = git_output(worktree, &["diff", "--binary", "--no-ext-diff"])?;
    if !output.status.success() {
        bail!(
            "git diff failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(output.stdout)
}

pub fn changed_files(worktree: &Path) -> Result<Vec<String>> {
    let output = git_text(
        worktree,
        &["status", "--porcelain=v1", "--untracked-files=all"],
    )?;
    Ok(output
        .lines()
        .filter_map(|line| line.get(3..))
        .map(str::to_owned)
        .collect())
}

pub fn apply_untrusted_patch(worktree: &Path, patch: &[u8]) -> Result<()> {
    if patch.is_empty() {
        bail!("api.patch_empty: submit_patch returned an empty patch");
    }
    if patch.len() > MAX_UNTRUSTED_PATCH_BYTES {
        bail!("api.patch_too_large: patch exceeds the 1 MiB limit");
    }
    let text =
        std::str::from_utf8(patch).context("api.patch_encoding: patch must be valid UTF-8")?;
    if text.as_bytes().contains(&0) || text.contains("GIT binary patch") {
        bail!("api.patch_binary_denied: binary patches are not allowed");
    }
    if !text.contains("diff --git ") {
        bail!("api.patch_format: patch must contain a git diff");
    }
    for denied in [
        "new file mode 120000",
        "old mode 120000",
        "new mode 120000",
        "new file mode 160000",
        "old mode 160000",
        "new mode 160000",
        "rename from ",
        "rename to ",
        "copy from ",
        "copy to ",
    ] {
        if text.lines().any(|line| line.starts_with(denied)) {
            bail!("api.patch_type_denied: links, submodules, renames, and copies are not allowed");
        }
    }
    if has_user_changes(&snapshot(worktree)?.status_porcelain_v2) {
        bail!("api.patch_worktree_dirty: isolated worktree must be clean before patch application");
    }
    run_git_apply(
        worktree,
        &["apply", "--check", "--whitespace=error-all", "-"],
        patch,
    )
    .context("api.patch_check_failed")?;
    run_git_apply(worktree, &["apply", "--whitespace=error-all", "-"], patch)
        .context("api.patch_apply_failed")
}

pub fn head(worktree: &Path) -> Result<String> {
    Ok(git_text(worktree, &["rev-parse", "HEAD"])?
        .trim()
        .to_owned())
}

pub fn cleanup_owned_task_worktrees(
    repository: &Path,
    implementation: &Worktree,
    verifier_path: &Path,
) -> Result<()> {
    if !implementation.branch.starts_with("garnish/task-") {
        bail!("cleanup refused a branch not owned by Garnish");
    }
    let before = snapshot(repository)?;
    let listing = git_text(repository, &["worktree", "list", "--porcelain"])?;
    let implementation_path = Path::new(&implementation.path);
    let canonical_implementation = implementation_path
        .canonicalize()
        .with_context(|| format!("resolving owned worktree {}", implementation.path))?;
    if !listing.contains(&format!("worktree {}", canonical_implementation.display()))
        || !listing.contains(&format!("branch refs/heads/{}", implementation.branch))
    {
        bail!("cleanup could not prove implementation worktree ownership");
    }
    if verifier_path.exists() {
        let canonical_verifier = verifier_path.canonicalize()?;
        if !listing.contains(&format!("worktree {}", canonical_verifier.display())) {
            bail!("cleanup could not prove verifier worktree ownership");
        }
        let verifier = canonical_verifier.to_string_lossy().into_owned();
        let removed = git_output(repository, &["worktree", "remove", "--force", &verifier])?;
        if !removed.status.success() {
            bail!(
                "git verifier worktree cleanup failed: {}",
                String::from_utf8_lossy(&removed.stderr).trim()
            );
        }
    }
    let implementation_arg = canonical_implementation.to_string_lossy().into_owned();
    let removed = git_output(
        repository,
        &["worktree", "remove", "--force", &implementation_arg],
    )?;
    if !removed.status.success() {
        bail!(
            "git implementation worktree cleanup failed: {}",
            String::from_utf8_lossy(&removed.stderr).trim()
        );
    }
    let deleted = git_output(
        repository,
        &["branch", "--delete", "--force", &implementation.branch],
    )?;
    if !deleted.status.success() {
        bail!(
            "git owned branch cleanup failed: {}",
            String::from_utf8_lossy(&deleted.stderr).trim()
        );
    }
    let after = snapshot(repository)?;
    if before.base_commit != after.base_commit
        || before.branch != after.branch
        || before.status_porcelain_v2 != after.status_porcelain_v2
    {
        bail!("source checkout changed during owned worktree cleanup");
    }
    Ok(())
}

pub fn run_argv(cwd: &Path, argv: &[String]) -> Result<Output> {
    let (program, args) = argv
        .split_first()
        .ok_or_else(|| anyhow::anyhow!("command argv is empty"))?;
    Command::new(program)
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("executing {program}"))
}

fn git_text(repository: &Path, args: &[&str]) -> Result<String> {
    let output = git_output(repository, args)?;
    if !output.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8(output.stdout)?)
}

fn git_output(repository: &Path, args: &[&str]) -> Result<Output> {
    Command::new("git")
        .args(args)
        .current_dir(repository)
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("executing git {}", args.join(" ")))
}

fn run_git_apply(repository: &Path, args: &[&str], input: &[u8]) -> Result<()> {
    let mut child = Command::new("git")
        .args(args)
        .current_dir(repository)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("executing git {}", args.join(" ")))?;
    child
        .stdin
        .take()
        .context("opening git apply stdin")?
        .write_all(input)?;
    let output = child.wait_with_output()?;
    if !output.status.success() {
        bail!("{}", String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn init_repo(path: &Path) {
        fs::create_dir_all(path).unwrap();
        Command::new("git")
            .args(["init", "-b", "main"])
            .current_dir(path)
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.email", "fixture@example.invalid"])
            .current_dir(path)
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.name", "Fixture"])
            .current_dir(path)
            .output()
            .unwrap();
        fs::write(path.join("README.md"), "fixture\n").unwrap();
        Command::new("git")
            .args(["add", "README.md"])
            .current_dir(path)
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "fixture"])
            .current_dir(path)
            .output()
            .unwrap();
    }

    #[test]
    fn creates_isolated_worktree_without_changing_source() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("source");
        init_repo(&source);
        let before = snapshot(&source).unwrap();
        let destination = dir.path().join("worktree");
        let worktree = create_task_worktree(&source, &destination, "01ABCDEF").unwrap();
        fs::write(destination.join("new.txt"), "new\n").unwrap();
        let after = snapshot(&source).unwrap();
        assert_eq!(before.status_porcelain_v2, after.status_porcelain_v2);
        assert_eq!(before.base_commit, after.base_commit);
        assert!(
            changed_files(Path::new(&worktree.path))
                .unwrap()
                .contains(&"new.txt".into())
        );
    }

    #[test]
    fn refuses_dirty_source() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("source");
        init_repo(&source);
        fs::write(source.join("dirty.txt"), "dirty").unwrap();
        assert!(create_task_worktree(&source, &dir.path().join("worktree"), "01ABC").is_err());
    }

    #[test]
    fn safely_reuses_only_a_clean_prepared_task_worktree() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("source");
        init_repo(&source);
        let destination = dir.path().join("worktree");
        let created = create_task_worktree(&source, &destination, "01REUSE").unwrap();
        let reused = create_or_reuse_task_worktree(&source, &destination, "01REUSE").unwrap();
        assert_eq!(reused.path, created.path);
        assert_eq!(reused.branch, created.branch);
        fs::write(destination.join("unexpected.txt"), "unsafe\n").unwrap();
        assert!(create_or_reuse_task_worktree(&source, &destination, "01REUSE").is_err());
    }

    #[test]
    fn verifier_applies_patch_to_separate_clean_worktree() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("source");
        init_repo(&source);
        let task = create_task_worktree(&source, &dir.path().join("task"), "01VERIFY").unwrap();
        fs::write(dir.path().join("task/result.txt"), "verified\n").unwrap();
        let task_patch = patch(Path::new(&task.path)).unwrap();
        let verifier = create_verification_worktree(
            &source,
            &dir.path().join("verifier"),
            &task.base_commit,
            &task_patch,
        )
        .unwrap();
        assert_eq!(
            fs::read_to_string(Path::new(&verifier.path).join("result.txt")).unwrap(),
            "verified\n"
        );
        assert!(!source.join("result.txt").exists());
    }

    #[test]
    fn untrusted_patch_is_applied_only_to_a_clean_worktree() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("source");
        init_repo(&source);
        let destination = dir.path().join("worktree");
        create_task_worktree(&source, &destination, "01PATCH").unwrap();
        let patch = b"diff --git a/result.txt b/result.txt\nnew file mode 100644\n--- /dev/null\n+++ b/result.txt\n@@ -0,0 +1 @@\n+done\n";
        apply_untrusted_patch(&destination, patch).unwrap();
        assert_eq!(
            fs::read_to_string(destination.join("result.txt")).unwrap(),
            "done\n"
        );
        assert!(!source.join("result.txt").exists());
    }

    #[test]
    fn untrusted_patch_rejects_binary_links_and_dirty_worktrees() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("source");
        init_repo(&source);
        let destination = dir.path().join("worktree");
        create_task_worktree(&source, &destination, "01DENY").unwrap();
        assert!(apply_untrusted_patch(&destination, b"GIT binary patch\0").is_err());
        let link = b"diff --git a/link b/link\nnew file mode 120000\n--- /dev/null\n+++ b/link\n@@ -0,0 +1 @@\n+target\n";
        assert!(apply_untrusted_patch(&destination, link).is_err());
        fs::write(destination.join("dirty.txt"), "dirty\n").unwrap();
        let patch = b"diff --git a/result.txt b/result.txt\nnew file mode 100644\n--- /dev/null\n+++ b/result.txt\n@@ -0,0 +1 @@\n+done\n";
        assert!(apply_untrusted_patch(&destination, patch).is_err());
    }

    #[test]
    fn owned_task_and_verifier_worktrees_and_branch_are_cleaned_without_source_change() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("source");
        init_repo(&source);
        let before = snapshot(&source).unwrap();
        let task = create_task_worktree(&source, &dir.path().join("task"), "01CLEAN").unwrap();
        fs::write(dir.path().join("task/result.txt"), "captured\n").unwrap();
        let task_patch = patch(Path::new(&task.path)).unwrap();
        let verifier_path = dir.path().join("verifier");
        create_verification_worktree(&source, &verifier_path, &task.base_commit, &task_patch)
            .unwrap();
        cleanup_owned_task_worktrees(&source, &task, &verifier_path).unwrap();
        assert!(!dir.path().join("task").exists());
        assert!(!verifier_path.exists());
        let branches = git_text(&source, &["branch", "--list", &task.branch]).unwrap();
        assert!(branches.trim().is_empty());
        let after = snapshot(&source).unwrap();
        assert_eq!(before.base_commit, after.base_commit);
        assert_eq!(before.branch, after.branch);
        assert_eq!(before.status_porcelain_v2, after.status_porcelain_v2);
    }
}
