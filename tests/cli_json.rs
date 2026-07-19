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
