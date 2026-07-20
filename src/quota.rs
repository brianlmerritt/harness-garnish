use crate::{
    adapters::{Invocation, discover_executable, run_invocation},
    process::ExitClassification,
};
use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    ffi::OsString,
    path::Path,
    sync::{Arc, atomic::AtomicBool},
    time::Duration as StdDuration,
};

pub const CODEXBAR_CONTRACT: &str = "codexbar-usage-json-v1";

// CodexBar can delegate collection to provider CLIs. Those nested processes need
// ordinary login-shell identity, terminal, locale, temporary-directory, and tool
// manager paths in addition to HOME/PATH. Keep credentials and unrelated process
// state out of the collector environment.
const CODEXBAR_ENVIRONMENT_KEYS: &[&str] = &[
    "HOME",
    "PATH",
    "XDG_CONFIG_HOME",
    "XDG_CACHE_HOME",
    "XDG_DATA_HOME",
    "USER",
    "LOGNAME",
    "SHELL",
    "TERM",
    "COLORTERM",
    "TMPDIR",
    "TMP",
    "TEMP",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "CLAUDE_CONFIG_DIR",
    "BUN_INSTALL",
    "NVM_DIR",
    "FNM_DIR",
    "VOLTA_HOME",
    "MISE_DATA_DIR",
    "ASDF_DATA_DIR",
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QuotaObservation {
    pub provider: String,
    pub account: String,
    pub surface: String,
    pub remaining_percent: Option<f64>,
    pub reserve_percent: f64,
    pub reset_at: Option<DateTime<Utc>>,
    pub source: String,
    pub confidence: String,
    pub unknown_reason: Option<String>,
    pub observed_at: DateTime<Utc>,
    pub valid_until: DateTime<Utc>,
    pub collector_contract: String,
    pub provider_version: Option<String>,
    pub payload_sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodexBarPayload {
    provider: String,
    #[serde(default)]
    version: Option<String>,
    source: String,
    usage: CodexBarUsage,
    #[serde(default)]
    account: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodexBarUsage {
    primary: Option<CodexBarWindow>,
    #[serde(default)]
    secondary: Option<CodexBarWindow>,
    #[serde(default)]
    tertiary: Option<CodexBarWindow>,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodexBarWindow {
    used_percent: f64,
    #[serde(default)]
    window_minutes: Option<u64>,
    #[serde(default)]
    resets_at: Option<DateTime<Utc>>,
}

pub fn codexbar_invocation(
    executable: Option<&Path>,
    cwd: &Path,
    provider: &str,
    collector_account: Option<&str>,
    source: &str,
) -> Result<Invocation> {
    validate_identity(provider, "provider")?;
    validate_identity(source, "source")?;
    if matches!(provider, "all" | "both") {
        bail!("CodexBar refresh requires one concrete provider");
    }
    if let Some(account) = collector_account {
        validate_identity(account, "CodexBar account")?;
    }
    if !matches!(source, "auto" | "web" | "cli" | "oauth" | "api") {
        bail!("CodexBar source must be auto, web, cli, oauth, or api");
    }
    let executable = executable
        .map(Path::to_path_buf)
        .or_else(|| discover_executable("codexbar"))
        .ok_or_else(|| anyhow!("codexbar executable not found"))?;
    let mut argv = vec![
        OsString::from("usage"),
        OsString::from("--provider"),
        OsString::from(provider),
        OsString::from("--source"),
        OsString::from(source),
        OsString::from("--format"),
        OsString::from("json"),
        OsString::from("--json-only"),
    ];
    if let Some(account) = collector_account {
        argv.extend([OsString::from("--account"), OsString::from(account)]);
    }
    let environment = codexbar_environment(cwd, |key| env::var(key).ok());
    Ok(Invocation {
        executable,
        argv,
        cwd: cwd.to_path_buf(),
        environment,
        stdin: vec![],
        structured_protocol: Some(CODEXBAR_CONTRACT.into()),
        timeout: StdDuration::from_secs(90),
        output_limit: 2 * 1024 * 1024,
    })
}

fn codexbar_environment(
    cwd: &Path,
    mut lookup: impl FnMut(&str) -> Option<String>,
) -> BTreeMap<String, String> {
    let mut environment = CODEXBAR_ENVIRONMENT_KEYS
        .iter()
        .filter_map(|key| lookup(key).map(|value| ((*key).to_owned(), value)))
        .collect::<BTreeMap<_, _>>();
    environment.insert("PWD".into(), cwd.to_string_lossy().into_owned());
    environment
}

#[allow(clippy::too_many_arguments)]
pub fn collect_codexbar(
    executable: Option<&Path>,
    cwd: &Path,
    provider: &str,
    account: &str,
    collector_account: Option<&str>,
    source: &str,
    reserve_percent: f64,
    valid_for: StdDuration,
) -> Result<Vec<QuotaObservation>> {
    if valid_for.is_zero() {
        bail!("quota observation validity must be greater than zero");
    }
    let invocation = codexbar_invocation(executable, cwd, provider, collector_account, source)?;
    let outcome = run_invocation(&invocation, Arc::new(AtomicBool::new(false)))?;
    if outcome.classification != ExitClassification::Success {
        let detail = codexbar_failure_detail(&outcome.stdout, &outcome.stderr);
        bail!(
            "CodexBar refresh failed ({:?}, exit {:?}): {detail}",
            outcome.classification,
            outcome.exit_code
        );
    }
    if outcome.stdout_truncated {
        bail!("CodexBar JSON output exceeded the 2 MiB safety limit");
    }
    parse_codexbar_usage(
        &outcome.stdout,
        provider,
        account,
        collector_account,
        reserve_percent,
        valid_for,
    )
}

pub fn parse_codexbar_usage(
    input: &[u8],
    expected_provider: &str,
    account: &str,
    expected_collector_account: Option<&str>,
    reserve_percent: f64,
    valid_for: StdDuration,
) -> Result<Vec<QuotaObservation>> {
    validate_identity(expected_provider, "provider")?;
    validate_identity(account, "account")?;
    validate_percentage(reserve_percent, "reserve")?;
    if valid_for.is_zero() {
        bail!("quota observation validity must be greater than zero");
    }
    let records = serde_json::Deserializer::from_slice(input)
        .into_iter::<serde_json::Value>()
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("parsing CodexBar usage JSON")?;
    let mut values = Vec::new();
    for record in records {
        match record {
            serde_json::Value::Object(ref object) if object.contains_key("provider") => {
                values.push(record);
            }
            serde_json::Value::Object(ref object)
                if object.contains_key("level")
                    && object.contains_key("message")
                    && object.contains_key("label") => {}
            serde_json::Value::Array(payloads) => values.extend(payloads),
            _ => bail!("CodexBar usage output contains an unexpected JSON record"),
        }
    }
    if values.is_empty() {
        bail!("CodexBar returned no usage payloads");
    }
    let mut matching = values
        .into_iter()
        .map(serde_json::from_value::<CodexBarPayload>)
        .collect::<std::result::Result<Vec<_>, _>>()?
        .into_iter()
        .filter(|payload| payload.provider == expected_provider)
        .collect::<Vec<_>>();
    if matching.len() != 1 {
        bail!(
            "CodexBar returned {} payloads for provider {expected_provider}; select one account explicitly",
            matching.len()
        );
    }
    let payload = matching.pop().expect("length checked");
    if let (Some(returned_account), Some(expected_account)) =
        (&payload.account, expected_collector_account)
        && returned_account != expected_account
    {
        bail!(
            "CodexBar account {returned_account} does not match selected collector account {expected_account}"
        );
    }
    if payload.source.trim().is_empty() {
        bail!("CodexBar payload source is empty");
    }
    let valid_for = Duration::from_std(valid_for)
        .map_err(|_| anyhow!("quota observation validity is too large"))?;
    let payload_sha256 = hex::encode(Sha256::digest(input));
    let mut surfaces = BTreeSet::new();
    let mut observations = Vec::new();
    let windows = [
        ("primary", payload.usage.primary),
        ("secondary", payload.usage.secondary),
        ("tertiary", payload.usage.tertiary),
    ];
    let missing_lanes = windows
        .iter()
        .filter_map(|(lane, window)| window.is_none().then_some(*lane))
        .collect::<BTreeSet<_>>();
    for (lane, window) in windows {
        let Some(window) = window else { continue };
        validate_percentage(window.used_percent, "used")?;
        if window.window_minutes == Some(0) {
            bail!("CodexBar {lane} windowMinutes must be greater than zero");
        }
        let surface = surface_key(lane, window.window_minutes);
        if !surfaces.insert(surface.clone()) {
            bail!("CodexBar returned duplicate quota surface {surface}");
        }
        observations.push(QuotaObservation {
            provider: payload.provider.clone(),
            account: account.into(),
            surface,
            remaining_percent: Some(100.0 - window.used_percent),
            reserve_percent,
            reset_at: window.resets_at,
            source: format!("codexbar:{}", payload.source),
            confidence: "provider_reported".into(),
            unknown_reason: None,
            observed_at: payload.usage.updated_at,
            valid_until: payload.usage.updated_at + valid_for,
            collector_contract: CODEXBAR_CONTRACT.into(),
            provider_version: payload.version.clone(),
            payload_sha256: payload_sha256.clone(),
        });
    }
    for &(lane, surface) in expected_surfaces(&payload.provider) {
        if missing_lanes.contains(lane) && surfaces.insert(surface.into()) {
            observations.push(QuotaObservation {
                provider: payload.provider.clone(),
                account: account.into(),
                surface: surface.into(),
                remaining_percent: None,
                reserve_percent,
                reset_at: None,
                source: format!("codexbar:{}", payload.source),
                confidence: "unknown".into(),
                unknown_reason: Some(format!("codexbar_missing_{lane}")),
                observed_at: payload.usage.updated_at,
                valid_until: payload.usage.updated_at + valid_for,
                collector_contract: CODEXBAR_CONTRACT.into(),
                provider_version: payload.version.clone(),
                payload_sha256: payload_sha256.clone(),
            });
        }
    }
    observations.sort_by(|left, right| left.surface.cmp(&right.surface));
    if observations.is_empty() {
        bail!("CodexBar payload contains no quota windows");
    }
    Ok(observations)
}

fn expected_surfaces(provider: &str) -> &'static [(&'static str, &'static str)] {
    match provider {
        "codex" | "claude" => &[("primary", "five_hour"), ("secondary", "weekly")],
        _ => &[],
    }
}

fn surface_key(lane: &str, minutes: Option<u64>) -> String {
    match minutes {
        Some(300) => "five_hour".into(),
        Some(10_080) => "weekly".into(),
        Some(43_200..=44_640) => "monthly".into(),
        Some(minutes) => format!("{lane}_{minutes}m"),
        None => lane.into(),
    }
}

fn validate_percentage(value: f64, label: &str) -> Result<()> {
    if !value.is_finite() || !(0.0..=100.0).contains(&value) {
        bail!("{label} percentage must be finite and between 0 and 100");
    }
    Ok(())
}

fn validate_identity(value: &str, label: &str) -> Result<()> {
    if value.trim().is_empty() || value.chars().any(char::is_whitespace) {
        bail!("{label} must be non-empty and contain no whitespace");
    }
    Ok(())
}

fn bounded_text(bytes: &[u8]) -> String {
    let value = String::from_utf8_lossy(bytes);
    value.chars().take(1_000).collect()
}

fn codexbar_failure_detail(stdout: &[u8], stderr: &[u8]) -> String {
    let mut provider_messages = Vec::new();
    let mut log_messages = Vec::new();
    for value in serde_json::Deserializer::from_slice(stdout)
        .into_iter::<serde_json::Value>()
        .filter_map(std::result::Result::ok)
    {
        collect_error_messages(&value, &mut provider_messages);
        if let Some(message) = value.get("message").and_then(serde_json::Value::as_str) {
            log_messages.push(message.to_owned());
        }
    }
    provider_messages
        .pop()
        .or_else(|| log_messages.pop())
        .map(|message| message.chars().take(1_000).collect())
        .or_else(|| {
            let detail = bounded_text(stderr);
            (!detail.trim().is_empty()).then_some(detail)
        })
        .unwrap_or_else(|| "no structured diagnostic returned".into())
}

fn collect_error_messages(value: &serde_json::Value, messages: &mut Vec<String>) {
    match value {
        serde_json::Value::Array(values) => {
            for value in values {
                collect_error_messages(value, messages);
            }
        }
        serde_json::Value::Object(_) => {
            if let Some(message) = value
                .pointer("/error/message")
                .and_then(serde_json::Value::as_str)
            {
                messages.push(message.to_owned());
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use tempfile::tempdir;

    const FIXTURE: &str = r#"{
      "provider": "codex",
      "version": "0.144.6",
      "source": "oauth",
      "account": "actual-account",
      "usage": {
        "primary": { "usedPercent": 28, "windowMinutes": 300, "resetsAt": "2026-07-20T19:15:00Z" },
        "secondary": { "usedPercent": 59, "windowMinutes": 10080, "resetsAt": "2026-07-25T17:00:00Z" },
        "tertiary": null,
        "updatedAt": "2026-07-20T18:10:22Z",
        "identity": { "providerID": "codex", "accountEmail": "user@example.com" }
      },
      "credits": { "remaining": 112.4 },
      "futureAdditiveField": true
    }"#;

    #[test]
    fn parser_normalizes_current_codexbar_contract_without_inventing_quota() {
        let observations = parse_codexbar_usage(
            FIXTURE.as_bytes(),
            "codex",
            "personal",
            None,
            20.0,
            StdDuration::from_secs(300),
        )
        .unwrap();
        assert_eq!(observations.len(), 2);
        assert_eq!(observations[0].surface, "five_hour");
        assert_eq!(observations[0].remaining_percent, Some(72.0));
        assert_eq!(observations[1].surface, "weekly");
        assert_eq!(observations[1].remaining_percent, Some(41.0));
        assert_eq!(observations[0].confidence, "provider_reported");
        assert_eq!(
            observations[0].valid_until,
            Utc.with_ymd_and_hms(2026, 7, 20, 18, 15, 22).unwrap()
        );
        assert_eq!(observations[0].payload_sha256.len(), 64);
    }

    #[test]
    fn parser_fails_closed_on_drift_ambiguity_and_invalid_percentages() {
        assert!(
            parse_codexbar_usage(
                br#"{"provider":"codex","source":"oauth"}"#,
                "codex",
                "personal",
                None,
                20.0,
                StdDuration::from_secs(300)
            )
            .is_err()
        );
        let invalid = FIXTURE.replace("\"usedPercent\": 28", "\"usedPercent\": 128");
        assert!(
            parse_codexbar_usage(
                invalid.as_bytes(),
                "codex",
                "personal",
                None,
                20.0,
                StdDuration::from_secs(300)
            )
            .is_err()
        );
        let array = format!("[{FIXTURE},{FIXTURE}]");
        assert!(
            parse_codexbar_usage(
                array.as_bytes(),
                "codex",
                "personal",
                None,
                20.0,
                StdDuration::from_secs(300)
            )
            .is_err()
        );
        assert!(
            parse_codexbar_usage(
                FIXTURE.as_bytes(),
                "codex",
                "personal",
                Some("different-account"),
                20.0,
                StdDuration::from_secs(300)
            )
            .is_err()
        );
    }

    #[test]
    fn missing_expected_window_is_preserved_as_unknown_evidence() {
        let fixture = FIXTURE.replace(
            r#""primary": { "usedPercent": 28, "windowMinutes": 300, "resetsAt": "2026-07-20T19:15:00Z" },"#,
            r#""primary": null,"#,
        );
        let observations = parse_codexbar_usage(
            fixture.as_bytes(),
            "codex",
            "personal",
            None,
            20.0,
            StdDuration::from_secs(300),
        )
        .unwrap();
        assert_eq!(observations.len(), 2);
        assert_eq!(observations[0].surface, "five_hour");
        assert_eq!(observations[0].remaining_percent, None);
        assert_eq!(
            observations[0].unknown_reason.as_deref(),
            Some("codexbar_missing_primary")
        );
        assert_eq!(observations[0].confidence, "unknown");
        assert_eq!(observations[1].surface, "weekly");
    }

    #[test]
    fn machine_log_prelude_is_accepted_but_unrecognized_records_fail_closed() {
        let logged = format!(
            "{{\"label\":\"codexbar.fixture\",\"level\":\"warning\",\"message\":\"bounded warning\"}}\n{FIXTURE}"
        );
        assert_eq!(
            parse_codexbar_usage(
                logged.as_bytes(),
                "codex",
                "personal",
                None,
                20.0,
                StdDuration::from_secs(300),
            )
            .unwrap()
            .len(),
            2
        );
        let unexpected = format!("{{\"message\":\"not a recognized log\"}}\n{FIXTURE}");
        assert!(
            parse_codexbar_usage(
                unexpected.as_bytes(),
                "codex",
                "personal",
                None,
                20.0,
                StdDuration::from_secs(300),
            )
            .is_err()
        );
    }

    #[test]
    fn failed_provider_message_is_extracted_from_json_stdout() {
        let stdout = br#"{"message":"diagnostic prelude"}
[{"provider":"claude","source":"auto","error":{"code":1,"message":"No Claude session key found in browser cookies.","kind":"provider"}}]"#;
        assert_eq!(
            codexbar_failure_detail(stdout, b""),
            "No Claude session key found in browser cookies."
        );
        assert_eq!(
            codexbar_failure_detail(b"", b"plain failure"),
            "plain failure"
        );
    }

    #[test]
    fn invocation_uses_argv_boundaries_and_whitelisted_environment() {
        let dir = tempdir().unwrap();
        let executable = dir.path().join("codexbar fixture");
        std::fs::write(&executable, "fixture").unwrap();
        let invocation = codexbar_invocation(
            Some(&executable),
            dir.path(),
            "codex",
            Some("work-account"),
            "oauth",
        )
        .unwrap();
        assert_eq!(invocation.executable, executable);
        assert_eq!(
            invocation.argv,
            [
                "usage",
                "--provider",
                "codex",
                "--source",
                "oauth",
                "--format",
                "json",
                "--json-only",
                "--account",
                "work-account"
            ]
            .map(OsString::from)
        );
        assert!(
            invocation
                .environment
                .keys()
                .all(|key| { key == "PWD" || CODEXBAR_ENVIRONMENT_KEYS.contains(&key.as_str()) })
        );
        assert_eq!(
            invocation.environment.get("PWD"),
            Some(&dir.path().to_string_lossy().into_owned())
        );
    }

    #[test]
    fn nested_cli_environment_keeps_runtime_context_but_not_credentials() {
        let dir = tempdir().unwrap();
        let supplied = BTreeMap::from([
            ("HOME", "/safe/home"),
            ("PATH", "/safe/bin"),
            ("USER", "fixture-user"),
            ("SHELL", "/bin/zsh"),
            ("TERM", "xterm-256color"),
            ("TMPDIR", "/safe/tmp"),
            ("LANG", "en_GB.UTF-8"),
            ("CLAUDE_CONFIG_DIR", "/safe/claude"),
            ("BUN_INSTALL", "/safe/bun"),
            ("ANTHROPIC_API_KEY", "must-not-leak"),
            ("CLAUDE_CODE_OAUTH_TOKEN", "must-not-leak"),
            ("AWS_SECRET_ACCESS_KEY", "must-not-leak"),
        ]);
        let environment = codexbar_environment(dir.path(), |key| {
            supplied.get(key).map(|value| (*value).to_owned())
        });

        for key in [
            "HOME",
            "PATH",
            "USER",
            "SHELL",
            "TERM",
            "TMPDIR",
            "LANG",
            "CLAUDE_CONFIG_DIR",
            "BUN_INSTALL",
            "PWD",
        ] {
            assert!(environment.contains_key(key), "missing {key}");
        }
        for key in [
            "ANTHROPIC_API_KEY",
            "CLAUDE_CODE_OAUTH_TOKEN",
            "AWS_SECRET_ACCESS_KEY",
        ] {
            assert!(!environment.contains_key(key), "leaked {key}");
        }
    }
}
