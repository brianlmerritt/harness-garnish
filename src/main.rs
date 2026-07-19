use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use clap::{Args, Parser, Subcommand, ValueEnum};
use harness_garnish::{Garnish, adapters::AgentKind, domain::NewTask};
use serde::Serialize;
use serde_json::json;
use std::{path::PathBuf, str::FromStr};

#[derive(Parser)]
#[command(
    name = "garnish",
    version,
    about = "Harness Garnish local control plane"
)]
struct Cli {
    #[arg(long, global = true, env = "GARNISH_DATA_DIR")]
    data_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Init,
    Doctor,
    Project {
        #[command(subcommand)]
        command: ProjectCommand,
    },
    Task {
        #[command(subcommand)]
        command: TaskCommand,
    },
    Quota {
        #[command(subcommand)]
        command: QuotaCommand,
    },
    Agent {
        #[command(subcommand)]
        command: AgentCommand,
    },
    Approval {
        #[command(subcommand)]
        command: ApprovalCommand,
    },
    Recover,
}

#[derive(Subcommand)]
enum ProjectCommand {
    Add(ProjectAdd),
    List,
    Link {
        #[arg(long)]
        parent: String,
        #[arg(long)]
        child: String,
        #[arg(long, default_value = "contains")]
        relationship: String,
    },
    Links,
}

#[derive(Args)]
struct ProjectAdd {
    #[arg(long)]
    slug: String,
    #[arg(long)]
    title: String,
    #[arg(long)]
    path: PathBuf,
}

#[derive(Subcommand)]
enum TaskCommand {
    Add(TaskAdd),
    List {
        #[arg(long)]
        project: Option<String>,
    },
    Show {
        id: String,
    },
    Dependency {
        id: String,
        #[arg(long)]
        depends_on: String,
    },
    Complete {
        id: String,
    },
    Route(TaskRoute),
    Run(TaskRoute),
}

#[derive(Args)]
struct TaskAdd {
    #[arg(long)]
    project: String,
    #[arg(long)]
    title: String,
    #[arg(long)]
    goal: String,
    #[arg(long, default_value = "user-requested work")]
    rationale: String,
    #[arg(long = "scope")]
    scope: Vec<String>,
    #[arg(long = "non-scope")]
    non_scope: Vec<String>,
    #[arg(long = "accept")]
    acceptance: Vec<String>,
    #[arg(long, help = "Verification command as a JSON argv array")]
    verify_argv: String,
    #[arg(long = "depends-on")]
    dependencies: Vec<String>,
    #[arg(long, default_value_t = 0)]
    priority: i64,
    #[arg(long, default_value_t = 1)]
    risk_class: u8,
    #[arg(long, default_value_t = 600)]
    estimated_seconds: u64,
    #[arg(long, default_value_t = 25)]
    uncertainty_percent: u8,
    #[arg(long, default_value_t = 300)]
    checkpoint_seconds: u64,
    #[arg(long)]
    fake_write_path: Option<String>,
    #[arg(long)]
    fake_write_content: Option<String>,
}

#[derive(Args)]
struct TaskRoute {
    id: String,
    #[arg(long, default_value = "fake")]
    adapter: String,
    #[arg(long, default_value = "fake")]
    provider: String,
    #[arg(long, default_value = "default")]
    account: String,
}

#[derive(Subcommand)]
enum QuotaCommand {
    Set(QuotaSet),
    Override(QuotaOverride),
    Status,
}

#[derive(Args)]
struct QuotaSet {
    #[arg(long)]
    provider: String,
    #[arg(long)]
    account: String,
    #[arg(long)]
    surface: String,
    #[arg(long)]
    remaining_percent: Option<f64>,
    #[arg(long, default_value_t = 20.0)]
    reserve_percent: f64,
    #[arg(long)]
    reset_at: Option<String>,
    #[arg(long, default_value = "manual")]
    source: String,
    #[arg(long)]
    unknown_reason: Option<String>,
}

#[derive(Args)]
struct QuotaOverride {
    #[arg(long)]
    provider: String,
    #[arg(long)]
    account: String,
    #[arg(long)]
    surface: String,
    #[arg(long)]
    remaining_percent: f64,
    #[arg(long)]
    reason: String,
    #[arg(long)]
    expires_at: Option<String>,
}

#[derive(Subcommand)]
enum AgentCommand {
    Probe,
    Invocation {
        #[arg(value_enum)]
        agent: AgentArg,
        #[arg(long)]
        cwd: PathBuf,
        #[arg(
            long,
            default_value = "Perform the bounded task described in the handoff."
        )]
        prompt: String,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum AgentArg {
    Codex,
    Claude,
    Antigravity,
}

impl From<AgentArg> for AgentKind {
    fn from(value: AgentArg) -> Self {
        match value {
            AgentArg::Codex => AgentKind::Codex,
            AgentArg::Claude => AgentKind::Claude,
            AgentArg::Antigravity => AgentKind::Antigravity,
        }
    }
}

#[derive(Subcommand)]
enum ApprovalCommand {
    Request {
        #[arg(long)]
        task: String,
        #[arg(long)]
        effect_class: u8,
        #[arg(long, help = "Canonical action JSON")]
        action: String,
        #[arg(long, default_value_t = 15)]
        expires_minutes: i64,
    },
    Approve {
        id: String,
    },
    Deny {
        id: String,
    },
    Consume {
        id: String,
        #[arg(long)]
        action: String,
    },
}

fn main() {
    if let Err(error) = run() {
        let payload = json!({
            "ok": false,
            "error": format!("{error:#}"),
        });
        eprintln!(
            "{}",
            serde_json::to_string(&payload).unwrap_or_else(|_| "{\"ok\":false}".into())
        );
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let data_dir = cli.data_dir.unwrap_or(default_data_dir()?);
    let mut garnish = Garnish::open(&data_dir)?;
    match cli.command {
        Command::Init => print_json(&json!({
            "ok": true,
            "data_dir": data_dir,
            "database": data_dir.join("state.db"),
        })),
        Command::Doctor => print_json(&garnish.doctor()),
        Command::Project { command } => match command {
            ProjectCommand::Add(args) => {
                let project = garnish.add_project(&args.slug, &args.title, &args.path)?;
                print_json(&project)
            }
            ProjectCommand::List => print_json(&garnish.projects()?),
            ProjectCommand::Link {
                parent,
                child,
                relationship,
            } => print_json(&garnish.link_projects(&parent, &child, &relationship)?),
            ProjectCommand::Links => print_json(&garnish.project_links()?),
        },
        Command::Task { command } => match command {
            TaskCommand::Add(args) => {
                let verification_argv: Vec<String> = serde_json::from_str(&args.verify_argv)
                    .context("--verify-argv must be a JSON string array")?;
                let task = garnish.add_task(&NewTask {
                    project_id: args.project,
                    title: args.title,
                    goal: args.goal,
                    rationale: args.rationale,
                    scope: args.scope,
                    non_scope: args.non_scope,
                    acceptance: args.acceptance,
                    verification_argv,
                    dependencies: args.dependencies,
                    priority: args.priority,
                    risk_class: args.risk_class,
                    estimated_seconds: args.estimated_seconds,
                    uncertainty_percent: args.uncertainty_percent,
                    checkpoint_seconds: args.checkpoint_seconds,
                    fake_write_path: args.fake_write_path,
                    fake_write_content: args.fake_write_content,
                })?;
                print_json(&task)
            }
            TaskCommand::List { project } => print_json(&garnish.tasks(project.as_deref())?),
            TaskCommand::Show { id } => print_json(&garnish.task(&id)?),
            TaskCommand::Dependency { id, depends_on } => {
                print_json(&garnish.add_dependency(&id, &depends_on)?)
            }
            TaskCommand::Complete { id } => print_json(&json!({
                "task_id": id,
                "status": "completed",
                "promoted": garnish.complete_task(&id)?,
            })),
            TaskCommand::Route(args) => print_json(&garnish.route_task(
                &args.id,
                &args.adapter,
                &args.provider,
                &args.account,
            )?),
            TaskCommand::Run(args) => print_json(&garnish.run_task(
                &args.id,
                &args.adapter,
                &args.provider,
                &args.account,
            )?),
        },
        Command::Quota { command } => match command {
            QuotaCommand::Set(args) => print_json(&garnish.set_quota(
                &args.provider,
                &args.account,
                &args.surface,
                args.remaining_percent,
                args.reserve_percent,
                parse_optional_time(args.reset_at.as_deref())?,
                &args.source,
                args.unknown_reason.as_deref(),
            )?),
            QuotaCommand::Override(args) => print_json(&garnish.override_quota(
                &args.provider,
                &args.account,
                &args.surface,
                args.remaining_percent,
                &args.reason,
                parse_optional_time(args.expires_at.as_deref())?,
            )?),
            QuotaCommand::Status => print_json(&garnish.quota()?),
        },
        Command::Agent { command } => match command {
            AgentCommand::Probe => print_json(&garnish.doctor().probes),
            AgentCommand::Invocation { agent, cwd, prompt } => {
                let kind: AgentKind = agent.into();
                let invocation = kind.invocation(&cwd, &prompt)?;
                print_json(&json!({
                    "agent": kind.key(),
                    "executable": invocation.executable,
                    "argv": invocation.argv.iter().map(|v| v.to_string_lossy()).collect::<Vec<_>>(),
                    "cwd": invocation.cwd,
                    "structured_protocol": invocation.structured_protocol,
                    "timeout_seconds": invocation.timeout.as_secs(),
                    "output_limit_bytes": invocation.output_limit,
                    "prompt_via_stdin": kind == AgentKind::Codex,
                }))
            }
        },
        Command::Approval { command } => match command {
            ApprovalCommand::Request {
                task,
                effect_class,
                action,
                expires_minutes,
            } => {
                if effect_class > 3 {
                    bail!("effect class must be 0..=3");
                }
                let action: serde_json::Value = serde_json::from_str(&action)?;
                let id = garnish.create_approval(&task, effect_class, &action, expires_minutes)?;
                print_json(&json!({"id": id, "status": "pending"}))
            }
            ApprovalCommand::Approve { id } => {
                garnish.decide_approval(&id, true)?;
                print_json(&json!({"id": id, "status": "approved"}))
            }
            ApprovalCommand::Deny { id } => {
                garnish.decide_approval(&id, false)?;
                print_json(&json!({"id": id, "status": "denied"}))
            }
            ApprovalCommand::Consume { id, action } => {
                let action: serde_json::Value = serde_json::from_str(&action)?;
                garnish.consume_approval(&id, &action)?;
                print_json(&json!({"id": id, "status": "consumed"}))
            }
        },
        Command::Recover => print_json(&json!({"recovered_task_ids": garnish.recover()?})),
    }
}

fn default_data_dir() -> Result<PathBuf> {
    directories::ProjectDirs::from("org", "Harness Garnish", "Harness Garnish")
        .map(|dirs| dirs.data_dir().to_path_buf())
        .ok_or_else(|| anyhow::anyhow!("could not determine platform data directory"))
}

fn parse_optional_time(value: Option<&str>) -> Result<Option<DateTime<Utc>>> {
    value
        .map(|value| {
            DateTime::from_str(value).with_context(|| format!("invalid RFC3339 timestamp: {value}"))
        })
        .transpose()
}

fn print_json(value: &impl Serialize) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}
