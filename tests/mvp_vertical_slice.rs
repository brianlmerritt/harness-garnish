use chrono::{Duration, Utc};
use harness_garnish::{
    Garnish,
    domain::{NewTask, TaskStatus},
    git,
};
use serde_json::Value;
use std::{fs, path::Path, process::Command};
use tempfile::tempdir;

fn fixture_repo(path: &Path, name: &str) {
    fs::create_dir_all(path).unwrap();
    for args in [
        vec!["init", "-b", "main"],
        vec!["config", "user.email", "fixture@example.invalid"],
        vec!["config", "user.name", "Fixture"],
    ] {
        assert!(
            Command::new("git")
                .args(args)
                .current_dir(path)
                .status()
                .unwrap()
                .success()
        );
    }
    fs::write(path.join("README.md"), format!("{name}\n")).unwrap();
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
            .args(["commit", "-m", "fixture"])
            .current_dir(path)
            .status()
            .unwrap()
            .success()
    );
}

fn task(
    project_id: String,
    title: &str,
    priority: i64,
    dependencies: Vec<String>,
    write_path: &str,
) -> NewTask {
    NewTask {
        project_id,
        title: title.into(),
        goal: format!("write {write_path}"),
        rationale: "deterministic MVP fixture".into(),
        scope: vec![write_path.into()],
        non_scope: vec!["remote Git".into(), "source checkout".into()],
        acceptance: vec![format!("{write_path} contains done")],
        verification_argv: vec!["grep".into(), "-q".into(), "done".into(), write_path.into()],
        dependencies,
        priority,
        risk_class: 1,
        estimated_seconds: 60,
        uncertainty_percent: 20,
        checkpoint_seconds: 60,
        day_affinity: harness_garnish::domain::DayAffinity::Both,
        fake_write_path: Some(write_path.into()),
        fake_write_content: Some("done\n".into()),
    }
}

fn meaningful_status(status: &str) -> Vec<&str> {
    status
        .lines()
        .filter(|line| !line.contains(".harness-garnish"))
        .collect()
}

#[test]
fn mvp_vertical_slice_produces_reviewable_machine_evidence() {
    let fixture = tempdir().unwrap();
    let portfolio_repo = fixture.path().join("portfolio");
    let implementation_repo = fixture.path().join("implementation");
    fixture_repo(&portfolio_repo, "portfolio");
    fixture_repo(&implementation_repo, "implementation");
    let portfolio_before = git::snapshot(&portfolio_repo).unwrap();
    let implementation_before = git::snapshot(&implementation_repo).unwrap();
    let data = fixture.path().join("garnish-data");

    let (prerequisite_id, dependent_id, portfolio_id, implementation_id) = {
        let mut garnish = Garnish::open(&data).unwrap();
        let portfolio = garnish
            .add_project("portfolio", "Portfolio", &portfolio_repo)
            .unwrap();
        let implementation = garnish
            .add_project("implementation", "Implementation", &implementation_repo)
            .unwrap();
        let link = garnish
            .link_projects(&portfolio.id, &implementation.id, "contains")
            .unwrap();
        assert_eq!(link.child_project_id, implementation.id);

        let prerequisite = garnish
            .add_task(&task(
                portfolio.id.clone(),
                "prepare handoff",
                10,
                vec![],
                "handoff-ready.txt",
            ))
            .unwrap();
        let dependent = garnish
            .add_task(&task(
                implementation.id.clone(),
                "implement bounded change",
                20,
                vec![prerequisite.id.clone()],
                "result.txt",
            ))
            .unwrap();
        assert_eq!(dependent.status, TaskStatus::Draft);
        assert!(
            garnish
                .route_task(&dependent.id, "fake-secondary", "fake", "test")
                .is_err()
        );

        for surface in ["five_hour", "weekly", "monthly"] {
            garnish
                .set_quota(
                    "fake",
                    "test",
                    surface,
                    Some(90.0),
                    20.0,
                    None,
                    "fixture",
                    None,
                )
                .unwrap();
        }
        let first = garnish
            .run_task(&prerequisite.id, "fake-primary", "fake", "test")
            .unwrap();
        assert_eq!(first.status, "review");
        (
            prerequisite.id,
            dependent.id,
            portfolio.id,
            implementation.id,
        )
    };

    // Reopening proves the checkpoint/review state is canonical in SQLite, not process memory.
    let mut garnish = Garnish::open(&data).unwrap();
    assert_eq!(
        garnish.task(&prerequisite_id).unwrap().status,
        TaskStatus::Review
    );
    let promoted = garnish.complete_task(&prerequisite_id).unwrap();
    assert_eq!(
        promoted.iter().map(|task| &task.id).collect::<Vec<_>>(),
        vec![&dependent_id]
    );
    assert_eq!(
        garnish.task(&dependent_id).unwrap().status,
        TaskStatus::Ready
    );

    let reset = Utc::now() + Duration::minutes(30);
    garnish
        .set_quota(
            "fake",
            "test",
            "five_hour",
            Some(5.0),
            20.0,
            Some(reset),
            "fixture",
            None,
        )
        .unwrap();
    let declined = garnish
        .route_task(&dependent_id, "fake-secondary", "fake", "test")
        .unwrap();
    assert!(!declined.allowed);
    assert!(declined.reason.contains("quota_headroom"));
    assert_eq!(declined.next_wake_at, Some(reset));
    assert_eq!(declined.quota.len(), 3);
    assert_eq!(declined.candidates.len(), 1);
    assert!(!declined.candidates[0].allowed);

    let overridden = garnish
        .override_quota(
            "fake",
            "test",
            "five_hour",
            90.0,
            "user changed subscription allocation mid-project",
            Some(Utc::now() + Duration::hours(1)),
        )
        .unwrap();
    assert_eq!(overridden.observed_remaining_percent, Some(5.0));
    assert_eq!(overridden.effective_remaining_percent, Some(90.0));
    let summary = garnish
        .run_task(&dependent_id, "fake-secondary", "fake", "test")
        .unwrap();
    assert_eq!(summary.status, "review");

    let manifest: Value =
        serde_json::from_slice(&fs::read(&summary.manifest_path).unwrap()).unwrap();
    assert_eq!(manifest["sandbox"]["secure_container"], true);
    assert_eq!(manifest["sandbox"]["network"], "none");
    assert_eq!(manifest["sandbox"]["container_socket_mounted"], false);
    let verification: Value =
        serde_json::from_slice(&fs::read(&summary.verification_path).unwrap()).unwrap();
    assert_eq!(verification["passed"], true);
    assert_ne!(verification["worktree"], summary.worktree);
    assert_eq!(verification["sandbox"]["secure_container"], true);
    let handoff: Value = serde_json::from_slice(&fs::read(&summary.handoff_path).unwrap()).unwrap();
    assert!(handoff.get("chain_of_thought").is_none());
    assert!(handoff.get("thought_process").is_none());
    assert!(
        fs::read(&summary.patch_path)
            .unwrap()
            .windows(b"result.txt".len())
            .any(|v| v == b"result.txt")
    );

    let events_path = Path::new(&summary.manifest_path)
        .parent()
        .unwrap()
        .join("events.jsonl");
    let events: Vec<Value> = fs::read_to_string(events_path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert!(
        events
            .windows(2)
            .all(|pair| pair[0]["sequence"].as_i64() < pair[1]["sequence"].as_i64())
    );
    assert!(
        events
            .iter()
            .all(|event| event["digest"].as_str().is_some())
    );

    drop(garnish);
    let garnish = Garnish::open(&data).unwrap();
    let projects = garnish.projects().unwrap();
    assert_eq!(projects.len(), 2);
    assert!(projects.iter().any(|project| project.id == portfolio_id));
    assert!(
        projects
            .iter()
            .any(|project| project.id == implementation_id)
    );
    assert_eq!(garnish.project_links().unwrap().len(), 1);
    let tasks = garnish.tasks(None).unwrap();
    assert_eq!(
        tasks[0].id, dependent_id,
        "global backlog priority changed across restart"
    );

    let portfolio_after = git::snapshot(&portfolio_repo).unwrap();
    let implementation_after = git::snapshot(&implementation_repo).unwrap();
    assert_eq!(portfolio_before.base_commit, portfolio_after.base_commit);
    assert_eq!(portfolio_before.branch, portfolio_after.branch);
    assert!(meaningful_status(&portfolio_after.status_porcelain_v2).is_empty());
    assert_eq!(
        implementation_before.base_commit,
        implementation_after.base_commit
    );
    assert_eq!(implementation_before.branch, implementation_after.branch);
    assert!(meaningful_status(&implementation_after.status_porcelain_v2).is_empty());
}
