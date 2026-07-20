use assert_cmd::cargo::cargo_bin_cmd;
use serde_json::Value;
use std::{fs, process::Command};
use tempfile::tempdir;

#[test]
fn api_budget_configuration_is_explicit_and_stable_json() {
    let dir = tempdir().unwrap();
    let period_start = (chrono::Utc::now() - chrono::Duration::minutes(5)).to_rfc3339();
    let period_end = (chrono::Utc::now() + chrono::Duration::days(30)).to_rfc3339();
    let repository = dir.path().join("repository");
    fs::create_dir(&repository).unwrap();
    assert!(
        Command::new("git")
            .args(["init", "-b", "main"])
            .current_dir(&repository)
            .status()
            .unwrap()
            .success()
    );
    let output = cargo_bin_cmd!("garnish")
        .args([
            "--data-dir",
            dir.path().join("state").to_str().unwrap(),
            "project",
            "add",
            "--slug",
            "fixture",
            "--title",
            "Fixture",
            "--path",
            repository.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(output.status.success());

    let output = cargo_bin_cmd!("garnish")
        .args([
            "--data-dir",
            dir.path().join("state").to_str().unwrap(),
            "api",
            "budget-set",
            "--project",
            "fixture",
            "--provider",
            "openai",
            "--account",
            "default",
            "--secret-reference",
            "env:OPENAI_API_KEY",
            "--currency",
            "USD",
            "--currency-limit-micros",
            "1000000",
            "--token-limit",
            "100000",
            "--request-limit",
            "100",
            "--period-start",
            period_start.as_str(),
            "--period-end",
            period_end.as_str(),
            "--model",
            "gpt-fixture",
            "--role",
            "planner",
            "--max-output-tokens",
            "4096",
            "--reason",
            "explicit fixture budget",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let budget: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(budget["provider"], "openai");
    assert_eq!(budget["currency_limit_micros"], 1_000_000);
    assert_eq!(budget["allowed_models"][0], "gpt-fixture");

    let output = cargo_bin_cmd!("garnish")
        .args([
            "--data-dir",
            dir.path().join("state").to_str().unwrap(),
            "api",
            "budget-status",
            "--project",
            "fixture",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let budgets: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(budgets.as_array().unwrap().len(), 1);
    assert_eq!(budgets[0]["enabled"], true);

    let output = cargo_bin_cmd!("garnish")
        .args([
            "--data-dir",
            dir.path().join("state").to_str().unwrap(),
            "task",
            "add",
            "--project",
            "fixture",
            "--title",
            "Planned API task",
            "--goal",
            "Exercise the durable request plan contract",
            "--accept",
            "plan is durable",
            "--verify-argv",
            "[\"true\"]",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let task: Value = serde_json::from_slice(&output.stdout).unwrap();
    let task_id = task["id"].as_str().unwrap();
    let output = cargo_bin_cmd!("garnish")
        .args([
            "--data-dir",
            dir.path().join("state").to_str().unwrap(),
            "task",
            "pin",
            task_id,
            "--adapter",
            "api",
            "--provider",
            "openai",
            "--account",
            "default",
            "--reason",
            "explicit paid API selection",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let output = cargo_bin_cmd!("garnish")
        .args([
            "--data-dir",
            dir.path().join("state").to_str().unwrap(),
            "api",
            "plan-set",
            "--task",
            task_id,
            "--provider",
            "openai",
            "--account",
            "default",
            "--model",
            "gpt-fixture",
            "--role",
            "planner",
            "--max-input-tokens",
            "10000",
            "--max-output-tokens",
            "512",
            "--reason",
            "bounded exact task request",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let plan: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(plan["task_id"], task_id);
    assert_eq!(plan["template_version"], "task-v1");
    assert_eq!(plan["request_digest"].as_str().unwrap().len(), 64);

    let output = cargo_bin_cmd!("garnish")
        .args([
            "--data-dir",
            dir.path().join("state").to_str().unwrap(),
            "api",
            "plan-status",
            "--task",
            task_id,
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let plans: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(plans.as_array().unwrap().len(), 1);
    assert_eq!(plans[0]["max_input_tokens"], 10_000);

    let output = cargo_bin_cmd!("garnish")
        .args([
            "--data-dir",
            dir.path().join("state").to_str().unwrap(),
            "api",
            "attempts",
            "--project",
            "fixture",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let attempts: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(attempts.as_array().unwrap().is_empty());
}

#[test]
fn api_model_price_evidence_is_append_only_and_stable_json() {
    let dir = tempdir().unwrap();
    let data = dir.path().join("state");
    let set = |reason: &str| {
        cargo_bin_cmd!("garnish")
            .args([
                "--data-dir",
                data.to_str().unwrap(),
                "api",
                "price-set",
                "--provider",
                "openai",
                "--account",
                "default",
                "--model",
                "model-fixture",
                "--currency",
                "USD",
                "--input-micros-per-million",
                "2000000",
                "--cached-input-micros-per-million",
                "500000",
                "--cache-creation-input-micros-per-million",
                "2500000",
                "--output-micros-per-million",
                "8000000",
                "--effective-from",
                "2026-07-20T00:00:00Z",
                "--source",
                "https://example.invalid/fixture-price-evidence",
                "--reason",
                reason,
            ])
            .output()
            .unwrap()
    };
    let first = set("initial fixture evidence");
    assert!(first.status.success());
    let first: Value = serde_json::from_slice(&first.stdout).unwrap();
    assert_eq!(first["input_micros_per_million"], 2_000_000);
    assert!(first["supersedes_id"].is_null());

    let second = set("replacement fixture evidence");
    assert!(second.status.success());
    let second: Value = serde_json::from_slice(&second.stdout).unwrap();
    assert_eq!(second["supersedes_id"], first["id"]);

    let status = cargo_bin_cmd!("garnish")
        .args(["--data-dir", data.to_str().unwrap(), "api", "price-status"])
        .output()
        .unwrap();
    assert!(status.status.success());
    let prices: Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(prices.as_array().unwrap().len(), 2);
}

#[test]
fn quota_usage_evidence_and_forecast_commands_are_stable_json() {
    let dir = tempdir().unwrap();
    let output = cargo_bin_cmd!("garnish")
        .args([
            "--data-dir",
            dir.path().to_str().unwrap(),
            "quota",
            "record-usage",
            "--evidence-id",
            "fixture-run-001",
            "--adapter",
            "codex",
            "--provider",
            "codex",
            "--account",
            "personal",
            "--surface",
            "five_hour",
            "--estimated-seconds",
            "600",
            "--consumed-percent",
            "3.5",
            "--source",
            "fixture-adapter",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let sample: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(sample["evidence_id"], "fixture-run-001");
    assert_eq!(sample["consumed_percent"], 3.5);

    let output = cargo_bin_cmd!("garnish")
        .args([
            "--data-dir",
            dir.path().to_str().unwrap(),
            "quota",
            "forecast",
            "--adapter",
            "codex",
            "--provider",
            "codex",
            "--account",
            "personal",
            "--estimated-seconds",
            "600",
            "--uncertainty-percent",
            "20",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let forecast: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(forecast["source"], "conservative_fallback");
    assert_eq!(forecast["sample_count"], 1);

    let output = cargo_bin_cmd!("garnish")
        .args([
            "--data-dir",
            dir.path().to_str().unwrap(),
            "quota",
            "samples",
            "--limit",
            "10",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let samples: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(samples.as_array().unwrap().len(), 1);
}

#[cfg(unix)]
#[test]
fn quota_refresh_codexbar_uses_bounded_machine_json_contract() {
    use std::{fs, os::unix::fs::PermissionsExt};

    let dir = tempdir().unwrap();
    let executable = dir.path().join("fake-codexbar");
    fs::write(
        &executable,
        r#"#!/bin/sh
printf '%s\n' '{"provider":"codex","version":"0.144.6","source":"oauth","usage":{"primary":{"usedPercent":28,"windowMinutes":300,"resetsAt":"2026-07-20T19:15:00Z"},"secondary":{"usedPercent":59,"windowMinutes":10080,"resetsAt":"2026-07-25T17:00:00Z"},"tertiary":null,"updatedAt":"2026-07-20T18:10:22Z"}}'
"#,
    )
    .unwrap();
    let mut permissions = fs::metadata(&executable).unwrap().permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&executable, permissions).unwrap();

    let output = cargo_bin_cmd!("garnish")
        .args([
            "--data-dir",
            dir.path().to_str().unwrap(),
            "quota",
            "refresh-codexbar",
            "--provider",
            "codex",
            "--account",
            "personal",
            "--source",
            "oauth",
            "--executable",
            executable.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    let surfaces = value.as_array().unwrap();
    assert_eq!(surfaces.len(), 2);
    assert_eq!(surfaces[0]["surface"], "five_hour");
    assert_eq!(surfaces[0]["observed_remaining_percent"], 72.0);
    assert_eq!(surfaces[0]["confidence"], "provider_reported");
    assert_eq!(surfaces[0]["collector_contract"], "codexbar-usage-json-v1");
    assert_eq!(surfaces[0]["payload_sha256"].as_str().unwrap().len(), 64);

    fs::write(
        &executable,
        r#"#!/bin/sh
printf '%s\n' '[{"provider":"claude","source":"auto","error":{"code":1,"kind":"provider","message":"No Claude session key found in browser cookies."}}]'
exit 1
"#,
    )
    .unwrap();
    let failure = cargo_bin_cmd!("garnish")
        .args([
            "--data-dir",
            dir.path().to_str().unwrap(),
            "quota",
            "refresh-codexbar",
            "--provider",
            "claude",
            "--account",
            "personal",
            "--executable",
            executable.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(failure.status.code(), Some(1));
    let error: Value = serde_json::from_slice(&failure.stderr).unwrap();
    assert!(
        error["error"]
            .as_str()
            .unwrap()
            .contains("No Claude session key")
    );

    let attempts = cargo_bin_cmd!("garnish")
        .args([
            "--data-dir",
            dir.path().to_str().unwrap(),
            "quota",
            "attempts",
        ])
        .output()
        .unwrap();
    assert!(attempts.status.success());
    let attempts: Value = serde_json::from_slice(&attempts.stdout).unwrap();
    let attempts = attempts.as_array().unwrap();
    assert_eq!(attempts.len(), 2);
    assert_eq!(attempts[0]["provider"], "claude");
    assert_eq!(attempts[0]["status"], "failed");
    assert!(
        attempts[0]["detail"]
            .as_str()
            .unwrap()
            .contains("No Claude session key")
    );
    assert_eq!(attempts[1]["provider"], "codex");
    assert_eq!(attempts[1]["status"], "succeeded");
}

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
    assert_eq!(value["schema_version"], 19);
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
