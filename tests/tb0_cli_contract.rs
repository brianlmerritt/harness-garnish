use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

const CONTRACT_JSON: &str = include_str!("../docs/contracts/tb0-cli-v1.json");

#[derive(Debug, Deserialize)]
struct Contract {
    contract: String,
    stability: String,
    global_options: Vec<String>,
    output_modes: Vec<String>,
    exit_codes: BTreeMap<String, String>,
    output_schemas: BTreeMap<String, Vec<String>>,
    commands: Vec<CommandSpec>,
    valid_fixtures: Vec<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct CommandSpec {
    path: String,
    #[serde(default)]
    positionals: Vec<String>,
    #[serde(default)]
    optional_positionals: Vec<String>,
    #[serde(default)]
    required_options: Vec<String>,
    #[serde(default)]
    optional_options: Vec<String>,
    #[serde(default)]
    flags: Vec<String>,
    effect: String,
    output: String,
}

fn contract() -> Contract {
    serde_json::from_str(CONTRACT_JSON).expect("TB-0 CLI contract must be valid JSON")
}

fn parse_contract_invocation<'a>(
    contract: &'a Contract,
    argv: &[String],
) -> Result<&'a CommandSpec, String> {
    if argv.is_empty() {
        return Err("command is required".into());
    }
    let mut candidates = contract
        .commands
        .iter()
        .filter_map(|spec| {
            let path = spec.path.split_whitespace().collect::<Vec<_>>();
            (argv.len() >= path.len()
                && argv
                    .iter()
                    .take(path.len())
                    .map(String::as_str)
                    .eq(path.iter().copied()))
            .then_some((path.len(), spec))
        })
        .collect::<Vec<_>>();
    candidates.sort_by_key(|(length, _)| std::cmp::Reverse(*length));
    let (path_length, spec) = candidates
        .first()
        .copied()
        .ok_or_else(|| format!("unknown command: {}", argv.join(" ")))?;

    if spec.path == "advanced" {
        return (argv.len() > 1)
            .then_some(spec)
            .ok_or_else(|| "advanced requires a legacy family".into());
    }

    let option_names = spec
        .required_options
        .iter()
        .chain(&spec.optional_options)
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let flag_names = spec
        .flags
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let mut seen_options = BTreeSet::new();
    let mut positional_count = 0;
    let mut index = path_length;
    while index < argv.len() {
        let token = argv[index].as_str();
        if flag_names.contains(token) {
            index += 1;
        } else if option_names.contains(token) {
            let value = argv
                .get(index + 1)
                .ok_or_else(|| format!("{token} requires a value"))?;
            if value.starts_with("--") {
                return Err(format!("{token} requires a value"));
            }
            seen_options.insert(token);
            index += 2;
        } else if token.starts_with("--") {
            return Err(format!("unknown option for {}: {token}", spec.path));
        } else {
            positional_count += 1;
            index += 1;
        }
    }

    if positional_count < spec.positionals.len() {
        return Err(format!("{} is missing a positional operand", spec.path));
    }
    if positional_count > spec.positionals.len() + spec.optional_positionals.len() {
        return Err(format!("{} has too many positional operands", spec.path));
    }
    for required in &spec.required_options {
        if !seen_options.contains(required.as_str()) {
            return Err(format!("{} requires {required}", spec.path));
        }
    }
    Ok(spec)
}

#[test]
fn tb0_contract_fixes_the_normal_families_and_machine_interface() {
    let contract = contract();
    assert_eq!(contract.contract, "garnish.cli/v1alpha1");
    assert_eq!(contract.stability, "tb0-frozen-before-implementation");
    assert_eq!(contract.output_modes, ["human", "json"]);
    assert_eq!(
        contract.global_options,
        ["--data-dir", "--output", "--no-color", "--quiet"]
    );
    assert_eq!(
        contract.exit_codes,
        BTreeMap::from([
            ("0".into(), "success".into()),
            ("2".into(), "usage".into()),
            ("3".into(), "validation".into()),
            ("4".into(), "not_found".into()),
            ("5".into(), "conflict".into()),
            ("6".into(), "denied".into()),
            ("7".into(), "unavailable".into()),
            ("8".into(), "external_uncertain".into()),
            ("9".into(), "internal".into()),
        ])
    );

    let actual_families = contract
        .commands
        .iter()
        .filter_map(|spec| {
            let family = spec.path.split_whitespace().next().unwrap();
            (family != "advanced").then_some(family)
        })
        .collect::<BTreeSet<_>>();
    let expected_families = BTreeSet::from([
        "agent",
        "approval",
        "calendar",
        "config",
        "doctor",
        "events",
        "init",
        "maintenance",
        "notification",
        "objective",
        "ops",
        "policy",
        "project",
        "quota",
        "route",
        "secret",
        "service",
        "status",
    ]);
    assert_eq!(actual_families, expected_families);
    for legacy in [
        "task",
        "api",
        "mcp",
        "schedule",
        "scheduler",
        "runtime",
        "ui",
    ] {
        assert!(!actual_families.contains(legacy));
    }
}

#[test]
fn tb0_command_specs_are_unique_bounded_and_schema_backed() {
    let contract = contract();
    let mut paths = BTreeSet::new();
    for spec in &contract.commands {
        assert!(
            paths.insert(&spec.path),
            "duplicate command path: {}",
            spec.path
        );
        assert!(
            contract.output_schemas.contains_key(&spec.output),
            "{} references missing output schema {}",
            spec.path,
            spec.output
        );
        assert!(
            !contract.output_schemas[&spec.output].is_empty(),
            "{} has an empty output schema",
            spec.path
        );
        if spec.effect == "material" {
            assert!(
                spec.flags.iter().any(|flag| flag == "--dry-run"),
                "material command lacks dry-run: {}",
                spec.path
            );
        }
        for option in spec
            .required_options
            .iter()
            .chain(&spec.optional_options)
            .chain(&spec.flags)
        {
            assert!(option.starts_with("--"), "invalid option in {}", spec.path);
        }
    }

    let forbidden_secret_argv = ["--secret-value", "--token", "--password", "--api-key"];
    for spec in contract
        .commands
        .iter()
        .filter(|spec| spec.path.starts_with("secret "))
    {
        for forbidden in forbidden_secret_argv {
            assert!(!spec.required_options.iter().any(|item| item == forbidden));
            assert!(!spec.optional_options.iter().any(|item| item == forbidden));
        }
    }
}

#[test]
fn tb0_valid_fixtures_parse_and_cover_every_normal_family() {
    let contract = contract();
    let mut covered = BTreeSet::new();
    for fixture in &contract.valid_fixtures {
        let parsed = parse_contract_invocation(&contract, fixture)
            .unwrap_or_else(|error| panic!("fixture `{}` failed: {error}", fixture.join(" ")));
        covered.insert(parsed.path.split_whitespace().next().unwrap());
    }
    let required = contract
        .commands
        .iter()
        .map(|spec| spec.path.split_whitespace().next().unwrap())
        .collect::<BTreeSet<_>>();
    assert_eq!(covered, required);
}

#[test]
fn tb0_contract_parser_rejects_missing_unknown_and_legacy_normal_commands() {
    let contract = contract();
    for invalid in [
        vec!["task", "run", "task-1"],
        vec!["project", "add"],
        vec!["calendar", "set", "default", "--pattern", "WWWOOBB"],
        vec!["objective", "add", "fixture", "--title", "Missing goal"],
        vec!["secret", "add", "fixture", "--token", "forbidden"],
        vec!["advanced"],
    ] {
        let argv = invalid.into_iter().map(str::to_owned).collect::<Vec<_>>();
        assert!(
            parse_contract_invocation(&contract, &argv).is_err(),
            "invalid fixture unexpectedly parsed: {}",
            argv.join(" ")
        );
    }
}
