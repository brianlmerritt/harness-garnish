use assert_cmd::cargo::cargo_bin_cmd;
use serde_json::Value;
use tempfile::tempdir;

#[test]
fn cli_success_and_failure_are_stable_json() {
    let dir = tempdir().unwrap();
    let success = cargo_bin_cmd!("garnish")
        .args(["--data-dir", dir.path().to_str().unwrap(), "init"])
        .output()
        .unwrap();
    assert!(success.status.success());
    let value: Value = serde_json::from_slice(&success.stdout).unwrap();
    assert_eq!(value["ok"], true);
    assert!(value["database"].as_str().unwrap().ends_with("state.db"));

    let schedule = cargo_bin_cmd!("garnish")
        .args([
            "--data-dir",
            dir.path().to_str().unwrap(),
            "schedule",
            "configure",
            "--slug",
            "uk-week",
            "--timezone",
            "Europe/London",
            "--weekly-pattern",
            "WWWWWOO",
        ])
        .output()
        .unwrap();
    assert!(schedule.status.success());
    let value: Value = serde_json::from_slice(&schedule.stdout).unwrap();
    assert_eq!(value["slug"], "uk-week");
    assert_eq!(value["timezone"], "Europe/London");
    assert_eq!(value["weekly_pattern"], "WWWWWOO");

    let registered = cargo_bin_cmd!("garnish")
        .args([
            "--data-dir",
            dir.path().to_str().unwrap(),
            "scheduler",
            "register",
            "--instance",
            "cli-test",
            "--hostname",
            "fixture",
        ])
        .output()
        .unwrap();
    assert!(registered.status.success());
    let leader = cargo_bin_cmd!("garnish")
        .args([
            "--data-dir",
            dir.path().to_str().unwrap(),
            "scheduler",
            "acquire-leader",
            "--instance",
            "cli-test",
            "--ttl-seconds",
            "30",
        ])
        .output()
        .unwrap();
    assert!(leader.status.success());
    let value: Value = serde_json::from_slice(&leader.stdout).unwrap();
    assert_eq!(value["instance_id"], "cli-test");
    assert_eq!(value["generation"], 1);

    let failure = cargo_bin_cmd!("garnish")
        .args([
            "--data-dir",
            dir.path().to_str().unwrap(),
            "task",
            "show",
            "missing-task",
        ])
        .output()
        .unwrap();
    assert_eq!(failure.status.code(), Some(1));
    assert!(failure.stdout.is_empty());
    let value: Value = serde_json::from_slice(&failure.stderr).unwrap();
    assert_eq!(value["ok"], false);
    assert!(value["error"].as_str().unwrap().contains("task not found"));
}

#[test]
fn scheduler_daemon_can_run_one_diagnostic_tick_and_stop_cleanly() {
    let dir = tempdir().unwrap();
    let output = cargo_bin_cmd!("garnish")
        .args([
            "--data-dir",
            dir.path().to_str().unwrap(),
            "scheduler",
            "daemon",
            "--instance",
            "cli-daemon",
            "--hostname",
            "fixture",
            "--poll-milliseconds",
            "1",
            "--leader-ttl-seconds",
            "2",
            "--claim-ttl-seconds",
            "2",
            "--max-ticks",
            "1",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["instance_id"], "cli-daemon");
    assert_eq!(value["ticks"], 1);
    assert_eq!(value["shutdown_reason"], "max_ticks");
}

#[test]
fn agent_status_reports_unknown_without_implicit_probe() {
    let dir = tempdir().unwrap();
    let output = cargo_bin_cmd!("garnish")
        .args([
            "--data-dir",
            dir.path().to_str().unwrap(),
            "agent",
            "status",
            "--at",
            "2026-07-20T12:00:00Z",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    let entries = value.as_array().unwrap();
    assert_eq!(entries.len(), 3);
    assert!(entries.iter().all(|entry| entry["freshness"] == "unknown"));
    assert_eq!(entries[0]["adapter"], "codex");
    assert_eq!(entries[1]["adapter"], "claude");
    assert_eq!(entries[2]["adapter"], "antigravity");
}

#[test]
fn operational_controls_status_and_backup_are_stable_json() {
    let dir = tempdir().unwrap();
    let data = dir.path().join("state");
    let pause = cargo_bin_cmd!("garnish")
        .args([
            "--data-dir",
            data.to_str().unwrap(),
            "ops",
            "pause",
            "--reason",
            "maintenance",
        ])
        .output()
        .unwrap();
    assert!(pause.status.success());
    let value: Value = serde_json::from_slice(&pause.stdout).unwrap();
    assert_eq!(value["pause_new_work"], true);

    let status = cargo_bin_cmd!("garnish")
        .args(["--data-dir", data.to_str().unwrap(), "ops", "status"])
        .output()
        .unwrap();
    assert!(status.status.success());
    let value: Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(value["control"]["pause_new_work"], true);

    let backup_path = dir.path().join("backups/state.db");
    let backup = cargo_bin_cmd!("garnish")
        .args([
            "--data-dir",
            data.to_str().unwrap(),
            "ops",
            "backup",
            "--output",
            backup_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        backup.status.success(),
        "{}",
        String::from_utf8_lossy(&backup.stderr)
    );
    let value: Value = serde_json::from_slice(&backup.stdout).unwrap();
    assert_eq!(value["integrity"], "ok");
    assert_eq!(value["schema_version"], 8);
    assert!(backup_path.exists());

    let resume = cargo_bin_cmd!("garnish")
        .args([
            "--data-dir",
            data.to_str().unwrap(),
            "ops",
            "resume",
            "--reason",
            "maintenance complete",
        ])
        .output()
        .unwrap();
    assert!(resume.status.success());
    let value: Value = serde_json::from_slice(&resume.stdout).unwrap();
    assert_eq!(value["pause_new_work"], false);
}
