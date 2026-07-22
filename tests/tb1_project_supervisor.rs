use assert_cmd::cargo::cargo_bin_cmd;
use serde_json::Value;
use std::{fs, path::Path, process::Command};
use tempfile::tempdir;

fn init_repository(path: &Path) {
    fs::create_dir_all(path).unwrap();
    assert!(
        Command::new("git")
            .args(["init", "-q", "--initial-branch=main"])
            .current_dir(path)
            .status()
            .unwrap()
            .success()
    );
    for (key, value) in [
        ("user.email", "fixture@example.invalid"),
        ("user.name", "Fixture"),
        ("commit.gpgsign", "false"),
        ("core.hooksPath", "/dev/null"),
    ] {
        assert!(
            Command::new("git")
                .args(["config", key, value])
                .current_dir(path)
                .status()
                .unwrap()
                .success()
        );
    }
    fs::write(path.join("README.md"), "fixture\n").unwrap();
    assert!(
        Command::new("git")
            .args(["add", "README.md"])
            .current_dir(path)
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new("git")
            .args(["commit", "-q", "-m", "fixture"])
            .current_dir(path)
            .status()
            .unwrap()
            .success()
    );
}

fn garnish(state: &Path, args: &[&str]) -> Value {
    let output = cargo_bin_cmd!("garnish")
        .arg("--data-dir")
        .arg(state)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "garnish {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "garnish {} returned invalid JSON ({error}): {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

#[test]
fn project_first_fixture_routes_hands_off_reviews_and_cleans_without_task_commands() {
    let fixture = tempdir().unwrap();
    let repository = fixture.path().join("repository");
    let state = fixture.path().join("state");
    init_repository(&repository);
    let source_head = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&repository)
        .output()
        .unwrap()
        .stdout;

    let initialized = garnish(
        &state,
        &[
            "init",
            "--calendar-pattern",
            "WWWOOBB",
            "--timezone",
            "Etc/UTC",
        ],
    );
    assert_eq!(
        initialized["supervisor"]["calendar"]["weekly_pattern"],
        "WWWOOBB"
    );
    assert_eq!(initialized["supervisor"]["execution_mode"], "fixture");
    let explained = garnish(&state, &["config", "explain", "calendar.default.pattern"]);
    assert_eq!(explained["effective_value"], "WWWOOBB");
    assert_eq!(explained["reschedule_required"], true);

    let project = garnish(
        &state,
        &[
            "project",
            "add",
            repository.to_str().unwrap(),
            "--slug",
            "fixture",
            "--title",
            "Fixture",
            "--affinity",
            "both",
        ],
    );
    assert_eq!(project["slug"], "fixture");
    assert_eq!(project["status"], "stopped");

    let objective = garnish(
        &state,
        &[
            "objective",
            "add",
            "fixture",
            "--title",
            "Exercise project supervision",
            "--goal",
            "Produce independently verified fixture evidence",
            "--accept",
            "fixture verification passes",
            "--fixture-write-path",
            "tb1-result.txt",
            "--fixture-write-content",
            "done",
        ],
    );
    assert_eq!(objective["status"], "ready");

    let started = garnish(&state, &["project", "start", "fixture"]);
    assert_eq!(started["status"], "active");
    let paused = garnish(
        &state,
        &[
            "project",
            "pause",
            "fixture",
            "--reason",
            "fixture lifecycle check",
        ],
    );
    assert_eq!(paused["status"], "paused");
    let resumed = garnish(&state, &["project", "resume", "fixture"]);
    assert_eq!(resumed["status"], "active");
    let codex_quota = garnish(
        &state,
        &[
            "agent",
            "configure",
            "codex-subscription",
            "quota.remaining_percent",
            "10",
            "--reason",
            "fixture low capacity",
        ],
    );
    assert_eq!(codex_quota["effective_remaining_percent"], 10.0);

    let service = garnish(&state, &["service", "run", "--max-cycles", "1"]);
    let cycle = &service["cycles"][0];
    assert_eq!(cycle["action"], "review");
    assert_eq!(cycle["selected_agent"], "claude-subscription");
    assert_eq!(cycle["previous_agent"], "codex-subscription");
    assert!(
        cycle["reason_code"]
            .as_str()
            .unwrap()
            .contains("quota.insufficient")
    );

    let status = garnish(&state, &["project", "status", "fixture"]);
    assert_eq!(status["objectives"][0]["status"], "review");
    assert_eq!(status["review_results"][0]["cleanup_status"], "complete");
    assert_eq!(status["cleanup"][0]["status"], "complete");
    assert_eq!(status["next_action"], "review_result");
    assert!(!repository.join("tb1-result.txt").exists());

    let worktrees = Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(&repository)
        .output()
        .unwrap();
    let worktrees = String::from_utf8(worktrees.stdout).unwrap();
    assert_eq!(worktrees.matches("worktree ").count(), 1);
    assert!(!worktrees.contains("garnish/task-"));
    let branches = Command::new("git")
        .args(["branch", "--list", "garnish/task-*"])
        .current_dir(&repository)
        .output()
        .unwrap();
    assert!(
        String::from_utf8(branches.stdout)
            .unwrap()
            .trim()
            .is_empty()
    );

    let reviews = garnish(&state, &["project", "review", "fixture"]);
    let result_id = reviews[0]["id"].as_str().unwrap();
    let applied = garnish(
        &state,
        &[
            "project",
            "apply",
            result_id,
            "--reason",
            "fixture accepted",
        ],
    );
    assert_eq!(applied["status"], "applied");
    assert_eq!(
        fs::read_to_string(repository.join("tb1-result.txt")).unwrap(),
        "done"
    );
    let final_status = garnish(&state, &["project", "status", "fixture"]);
    assert_eq!(final_status["objectives"][0]["status"], "completed");
    let stopped = garnish(
        &state,
        &["project", "stop", "fixture", "--reason", "fixture complete"],
    );
    assert_eq!(stopped["status"], "stopped");

    let final_head = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&repository)
        .output()
        .unwrap()
        .stdout;
    assert_eq!(source_head, final_head);

    let advanced = garnish(
        &state,
        &["advanced", "task", "list", "--project", "fixture"],
    );
    assert_eq!(advanced.as_array().unwrap().len(), 1);
}

#[test]
fn calendar_preview_applies_work_non_work_and_shared_days() {
    let fixture = tempdir().unwrap();
    let repository = fixture.path().join("repository");
    let state = fixture.path().join("state");
    init_repository(&repository);
    garnish(
        &state,
        &[
            "init",
            "--calendar-pattern",
            "WWWOOBB",
            "--timezone",
            "Etc/UTC",
        ],
    );
    garnish(
        &state,
        &[
            "project",
            "add",
            repository.to_str().unwrap(),
            "--slug",
            "work-fixture",
            "--affinity",
            "work",
        ],
    );
    let preview = garnish(
        &state,
        &[
            "calendar",
            "preview",
            "default",
            "--project",
            "work-fixture",
            "--from",
            "2026-07-20",
            "--days",
            "7",
        ],
    );
    assert_eq!(preview["days"][3]["day_kind"], "O");
    assert_eq!(preview["days"][3]["eligible"], false);
    assert_eq!(preview["days"][5]["day_kind"], "B");
    assert_eq!(preview["days"][5]["eligible"], true);

    garnish(
        &state,
        &[
            "calendar",
            "exception",
            "set",
            "default",
            "2026-07-23",
            "B",
            "--reason",
            "shared fixture day",
        ],
    );
    let shared_exception = garnish(
        &state,
        &[
            "calendar",
            "preview",
            "default",
            "--project",
            "work-fixture",
            "--from",
            "2026-07-23",
            "--days",
            "1",
        ],
    );
    assert_eq!(shared_exception["days"][0]["day_kind"], "B");
    assert_eq!(shared_exception["days"][0]["eligible"], true);
    garnish(
        &state,
        &[
            "calendar",
            "exception",
            "remove",
            "default",
            "2026-07-23",
            "--reason",
            "fixture complete",
        ],
    );
}

#[test]
fn bounded_service_cycle_selects_the_eligible_project_without_task_ids() {
    let fixture = tempdir().unwrap();
    let work_repository = fixture.path().join("work-repository");
    let non_work_repository = fixture.path().join("non-work-repository");
    let state = fixture.path().join("state");
    init_repository(&work_repository);
    init_repository(&non_work_repository);
    garnish(
        &state,
        &[
            "init",
            "--calendar-pattern",
            "WWWOOBB",
            "--timezone",
            "Etc/UTC",
        ],
    );
    let work = garnish(
        &state,
        &[
            "project",
            "add",
            work_repository.to_str().unwrap(),
            "--slug",
            "work",
            "--affinity",
            "work",
        ],
    );
    let non_work = garnish(
        &state,
        &[
            "project",
            "add",
            non_work_repository.to_str().unwrap(),
            "--slug",
            "non-work",
            "--affinity",
            "non-work",
        ],
    );
    for project in ["work", "non-work"] {
        garnish(
            &state,
            &[
                "objective",
                "add",
                project,
                "--title",
                "Scheduled fixture",
                "--goal",
                "Exercise project calendar selection",
                "--accept",
                "fixture verification passes",
            ],
        );
        garnish(&state, &["project", "start", project]);
    }

    let off_day = garnish(
        &state,
        &[
            "service",
            "run",
            "--max-cycles",
            "1",
            "--at",
            "2026-07-23T12:00:00Z",
        ],
    );
    assert_eq!(off_day["cycles"][0]["project_id"], non_work["id"]);
    assert_eq!(
        off_day["cycles"][0]["selected_agent"], "codex-subscription",
        "{off_day:#}"
    );

    let work_day = garnish(
        &state,
        &[
            "service",
            "run",
            "--max-cycles",
            "1",
            "--at",
            "2026-07-20T12:00:00Z",
        ],
    );
    assert_eq!(work_day["cycles"][0]["project_id"], work["id"]);
    assert_eq!(
        work_day["cycles"][0]["selected_agent"],
        "codex-subscription"
    );
    assert_eq!(
        garnish(&state, &["project", "status", "work"])["objectives"][0]["status"],
        "review"
    );
    assert_eq!(
        garnish(&state, &["project", "status", "non-work"])["objectives"][0]["status"],
        "review"
    );
}

#[test]
fn failed_fixture_attempt_is_blocked_and_its_owned_git_resources_are_removed() {
    let fixture = tempdir().unwrap();
    let repository = fixture.path().join("repository");
    let state = fixture.path().join("state");
    init_repository(&repository);
    garnish(&state, &["init"]);
    garnish(
        &state,
        &[
            "project",
            "add",
            repository.to_str().unwrap(),
            "--slug",
            "failure-fixture",
            "--affinity",
            "both",
        ],
    );
    garnish(
        &state,
        &[
            "objective",
            "add",
            "failure-fixture",
            "--title",
            "Reject an escaping fixture write",
            "--goal",
            "Prove failed attempts are cleaned",
            "--accept",
            "unsafe path is rejected",
            "--fixture-write-path",
            "../escape.txt",
            "--fixture-write-content",
            "done",
        ],
    );
    garnish(&state, &["project", "start", "failure-fixture"]);
    let service = garnish(&state, &["service", "run", "--max-cycles", "1"]);
    assert_eq!(service["cycles"][0]["action"], "blocked");
    assert!(
        service["cycles"][0]["reason_code"]
            .as_str()
            .unwrap()
            .contains("cleanup:complete")
    );
    let status = garnish(&state, &["project", "status", "failure-fixture"]);
    assert_eq!(status["objectives"][0]["status"], "blocked");
    assert_eq!(status["cleanup"][0]["status"], "complete");
    assert!(!fixture.path().join("escape.txt").exists());

    let worktrees = Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(&repository)
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8(worktrees.stdout)
            .unwrap()
            .matches("worktree ")
            .count(),
        1
    );
    let branches = Command::new("git")
        .args(["branch", "--list", "garnish/task-*"])
        .current_dir(&repository)
        .output()
        .unwrap();
    assert!(
        String::from_utf8(branches.stdout)
            .unwrap()
            .trim()
            .is_empty()
    );
}
