use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use clap::{Args, Parser, Subcommand, ValueEnum};
use harness_garnish::{
    Garnish,
    adapters::AgentKind,
    domain::{DayAffinity, DayKind, NewApiBudget, NewTask, RouteTarget, SchedulerDaemonConfig},
    web_ui::{UiServerConfig, serve_ui},
};
use serde::Serialize;
use serde_json::json;
use std::{
    io::Write,
    path::PathBuf,
    str::FromStr,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration as StdDuration,
};

static SCHEDULER_SHUTDOWN: AtomicBool = AtomicBool::new(false);

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
    Api {
        #[command(subcommand)]
        command: ApiCommand,
    },
    Schedule {
        #[command(subcommand)]
        command: ScheduleCommand,
    },
    Scheduler {
        #[command(subcommand)]
        command: SchedulerCommand,
    },
    Runtime {
        #[command(subcommand)]
        command: RuntimeCommand,
    },
    Ops {
        #[command(subcommand)]
        command: OpsCommand,
    },
    Notification {
        #[command(subcommand)]
        command: NotificationCommand,
    },
    Agent {
        #[command(subcommand)]
        command: AgentCommand,
    },
    Approval {
        #[command(subcommand)]
        command: ApprovalCommand,
    },
    Ui {
        #[command(subcommand)]
        command: UiCommand,
    },
    Recover,
}

#[derive(Subcommand)]
enum UiCommand {
    Serve(UiServeArgs),
}

#[derive(Args)]
struct UiServeArgs {
    #[arg(long, default_value_t = 7467)]
    port: u16,
    #[arg(
        long,
        help = "Stop after this many HTTP requests; intended for bounded diagnostics"
    )]
    max_requests: Option<usize>,
}

#[derive(Subcommand)]
enum ProjectCommand {
    Add(ProjectAdd),
    List,
    Pause {
        #[arg(long)]
        project: String,
        #[arg(long)]
        reason: String,
    },
    Resume {
        #[arg(long)]
        project: String,
        #[arg(long)]
        reason: String,
    },
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
    Add(Box<TaskAdd>),
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
    Pin {
        id: String,
        #[arg(long)]
        adapter: String,
        #[arg(long)]
        provider: String,
        #[arg(long)]
        account: String,
        #[arg(long)]
        reason: String,
    },
    Unpin {
        id: String,
        #[arg(long)]
        reason: String,
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
    #[arg(long, default_value = "B", help = "Day affinity: W, O, or B")]
    day_affinity: String,
    #[arg(
        long,
        help = "Optional RFC3339 deadline; the task is excluded after it passes"
    )]
    deadline_at: Option<String>,
    #[arg(long = "requires-capability")]
    required_capabilities: Vec<String>,
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
    #[command(
        name = "refresh-codexbar",
        about = "Fetch CodexBar JSON; may access provider authentication and the network"
    )]
    RefreshCodexbar(QuotaRefreshCodexbar),
    #[command(
        name = "record-usage",
        about = "Record explicit per-run usage telemetry; never inferred from account deltas"
    )]
    RecordUsage(QuotaRecordUsage),
    Forecast(QuotaForecast),
    Samples {
        #[arg(long, default_value_t = 100)]
        limit: usize,
    },
    Attempts,
    Reservations,
    Status,
}

#[derive(Subcommand)]
enum ApiCommand {
    BudgetSet(Box<ApiBudgetSet>),
    BudgetStatus {
        #[arg(long)]
        project: Option<String>,
    },
    Reservations {
        #[arg(long)]
        project: Option<String>,
    },
    Spend {
        #[arg(long)]
        project: Option<String>,
    },
}

#[derive(Args)]
struct ApiBudgetSet {
    #[arg(long)]
    project: String,
    #[arg(long)]
    provider: String,
    #[arg(long, default_value = "default")]
    account: String,
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    enabled: bool,
    #[arg(
        long,
        help = "Non-secret env:NAME, keychain:SERVICE/ACCOUNT, or file:/absolute/path reference"
    )]
    secret_reference: String,
    #[arg(long)]
    currency: Option<String>,
    #[arg(long)]
    currency_limit_micros: Option<u64>,
    #[arg(long)]
    token_limit: Option<u64>,
    #[arg(long)]
    request_limit: Option<u64>,
    #[arg(long, help = "RFC3339 inclusive period start")]
    period_start: String,
    #[arg(long, help = "RFC3339 exclusive period end")]
    period_end: String,
    #[arg(long = "model", required = true)]
    allowed_models: Vec<String>,
    #[arg(long = "tool")]
    allowed_tools: Vec<String>,
    #[arg(long = "role", required = true)]
    allowed_roles: Vec<String>,
    #[arg(long)]
    max_output_tokens: u64,
    #[arg(long, default_value_t = 0)]
    max_retries: u32,
    #[arg(long, default_value_t = 1)]
    max_concurrent_requests: u32,
    #[arg(long)]
    reason: String,
}

#[derive(Args)]
struct QuotaRecordUsage {
    #[arg(
        long,
        help = "Stable collector/run evidence identifier used for deduplication"
    )]
    evidence_id: String,
    #[arg(long)]
    adapter: String,
    #[arg(long)]
    provider: String,
    #[arg(long, default_value = "default")]
    account: String,
    #[arg(long)]
    surface: String,
    #[arg(long)]
    estimated_seconds: u64,
    #[arg(long)]
    consumed_percent: f64,
    #[arg(long, help = "Collector or evidence source name")]
    source: String,
    #[arg(
        long,
        default_value = "collector_measured",
        help = "provider_reported, collector_measured, or user_reported"
    )]
    confidence: String,
    #[arg(long, help = "Optional RFC3339 evidence time; defaults to now")]
    observed_at: Option<String>,
}

#[derive(Args)]
struct QuotaForecast {
    #[arg(long)]
    adapter: String,
    #[arg(long)]
    provider: String,
    #[arg(long, default_value = "default")]
    account: String,
    #[arg(long)]
    estimated_seconds: u64,
    #[arg(long, default_value_t = 25)]
    uncertainty_percent: u8,
}

#[derive(Args)]
struct QuotaRefreshCodexbar {
    #[arg(long, help = "Concrete CodexBar provider ID (not all or both)")]
    provider: String,
    #[arg(long, default_value = "default", help = "Garnish account identity")]
    account: String,
    #[arg(
        long,
        help = "Optional CodexBar account selector; omit for its current/default account"
    )]
    collector_account: Option<String>,
    #[arg(long, default_value = "auto", help = "auto, web, cli, oauth, or api")]
    source: String,
    #[arg(long, default_value_t = 20.0)]
    reserve_percent: f64,
    #[arg(long, default_value_t = 300)]
    valid_seconds: u64,
    #[arg(
        long,
        help = "Explicit CodexBar executable path; otherwise PATH is searched"
    )]
    executable: Option<PathBuf>,
}

#[derive(Subcommand)]
enum ScheduleCommand {
    Configure {
        #[arg(long, default_value = "default")]
        slug: String,
        #[arg(long)]
        timezone: String,
        #[arg(long, default_value = "WWWWWOO")]
        weekly_pattern: String,
    },
    Assign {
        #[arg(long)]
        project: String,
        #[arg(long)]
        calendar: String,
    },
    Exception {
        #[arg(long, default_value = "default")]
        calendar: String,
        #[arg(long, help = "Local date in YYYY-MM-DD form")]
        date: String,
        #[arg(long, help = "Day kind: W or O")]
        kind: String,
        #[arg(long)]
        reason: String,
    },
    Evaluate {
        #[arg(long)]
        task: String,
        #[arg(long, help = "Optional RFC3339 instant; defaults to now")]
        at: Option<String>,
    },
    Preview {
        #[arg(long, default_value = "fake")]
        adapter: String,
        #[arg(long, default_value = "fake")]
        provider: String,
        #[arg(long, default_value = "default")]
        account: String,
        #[arg(long, help = "Optional RFC3339 instant; defaults to now")]
        at: Option<String>,
    },
}

#[derive(Subcommand)]
enum SchedulerCommand {
    Daemon(SchedulerDaemonArgs),
    Register {
        #[arg(long)]
        instance: String,
        #[arg(long, default_value = "local")]
        hostname: String,
    },
    AcquireLeader {
        #[arg(long)]
        instance: String,
        #[arg(long, default_value_t = 30)]
        ttl_seconds: u64,
    },
    Heartbeat {
        #[arg(long)]
        instance: String,
        #[arg(long)]
        generation: i64,
        #[arg(long, default_value_t = 30)]
        ttl_seconds: u64,
    },
    Tick {
        #[arg(long)]
        instance: String,
        #[arg(long)]
        generation: i64,
        #[arg(long, default_value = "fake")]
        adapter: String,
        #[arg(long, default_value = "fake")]
        provider: String,
        #[arg(long, default_value = "default")]
        account: String,
        #[arg(
            long = "candidate",
            value_name = "ADAPTER:PROVIDER:ACCOUNT",
            help = "Repeatable route candidate; when present these replace the legacy adapter/provider/account target"
        )]
        candidates: Vec<String>,
        #[arg(long, default_value_t = 1)]
        max_active: usize,
        #[arg(long, default_value_t = 1)]
        max_active_per_adapter: usize,
        #[arg(long, default_value_t = 1)]
        max_active_per_account: usize,
        #[arg(long, default_value_t = 300)]
        claim_ttl_seconds: u64,
        #[arg(long, help = "Optional RFC3339 instant; defaults to now")]
        at: Option<String>,
    },
    Recover {
        #[arg(long, help = "Optional RFC3339 instant; defaults to now")]
        at: Option<String>,
    },
    Stop {
        #[arg(long)]
        instance: String,
    },
    Wakes,
}

#[derive(Subcommand)]
enum RuntimeCommand {
    Runs {
        #[arg(long)]
        task: String,
    },
    Checkpoint {
        #[arg(long)]
        run: String,
        #[arg(long, default_value = "fake")]
        provider: String,
        #[arg(long, default_value = "default")]
        account: String,
        #[arg(long, help = "Optional RFC3339 instant; defaults to now")]
        at: Option<String>,
    },
    Cancel {
        #[arg(long)]
        run: String,
        #[arg(long)]
        reason: String,
    },
    RetryState {
        #[arg(long)]
        task: String,
    },
    RetryLimit {
        #[arg(long)]
        task: String,
        #[arg(long)]
        limit: u32,
    },
    Circuits,
}

#[derive(Subcommand)]
enum OpsCommand {
    Status,
    Pause {
        #[arg(long)]
        reason: String,
    },
    Resume {
        #[arg(long)]
        reason: String,
    },
    EmergencyStop {
        #[arg(long)]
        reason: String,
    },
    Diagnostics,
    Backup {
        #[arg(long)]
        output: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum NotificationCommand {
    List {
        #[arg(long)]
        all: bool,
        #[arg(long, default_value_t = 50)]
        limit: usize,
    },
    Acknowledge {
        id: String,
    },
}

#[derive(Args)]
struct SchedulerDaemonArgs {
    #[arg(long)]
    instance: String,
    #[arg(long, default_value = "local")]
    hostname: String,
    #[arg(long, default_value = "fake")]
    adapter: String,
    #[arg(long, default_value = "fake")]
    provider: String,
    #[arg(long, default_value = "default")]
    account: String,
    #[arg(
        long = "candidate",
        value_name = "ADAPTER:PROVIDER:ACCOUNT",
        help = "Repeatable route candidate; when present these replace the legacy adapter/provider/account target"
    )]
    candidates: Vec<String>,
    #[arg(long, default_value_t = 1)]
    max_active: usize,
    #[arg(long, default_value_t = 1)]
    max_active_per_adapter: usize,
    #[arg(long, default_value_t = 1)]
    max_active_per_account: usize,
    #[arg(long, default_value_t = 1000)]
    poll_milliseconds: u64,
    #[arg(long, default_value_t = 30)]
    leader_ttl_seconds: u64,
    #[arg(long, default_value_t = 300)]
    claim_ttl_seconds: u64,
    #[arg(
        long,
        help = "Stop cleanly after this many ticks (primarily for diagnostics)"
    )]
    max_ticks: Option<usize>,
    #[arg(
        long,
        help = "Execute claimed work with the quota-free fake adapter (real agents remain disabled)"
    )]
    execute_fake: bool,
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
    Refresh {
        #[arg(long, default_value_t = 300)]
        valid_seconds: u64,
    },
    Status {
        #[arg(long, help = "Optional RFC3339 instant; defaults to now")]
        at: Option<String>,
    },
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
            ProjectCommand::Pause { project, reason } => {
                print_json(&garnish.set_project_scheduler_pause(&project, true, &reason)?)
            }
            ProjectCommand::Resume { project, reason } => {
                print_json(&garnish.set_project_scheduler_pause(&project, false, &reason)?)
            }
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
                    day_affinity: DayAffinity::from_str(&args.day_affinity)?,
                    deadline_at: parse_optional_time(args.deadline_at.as_deref())?,
                    required_capabilities: args.required_capabilities,
                    pinned_adapter: None,
                    pinned_provider: None,
                    pinned_account: None,
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
            TaskCommand::Pin {
                id,
                adapter,
                provider,
                account,
                reason,
            } => print_json(
                &garnish.set_task_route_pin(&id, &adapter, &provider, &account, &reason)?,
            ),
            TaskCommand::Unpin { id, reason } => {
                print_json(&garnish.clear_task_route_pin(&id, &reason)?)
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
            QuotaCommand::RefreshCodexbar(args) => print_json(&garnish.refresh_quota_codexbar(
                args.executable.as_deref(),
                &args.provider,
                &args.account,
                args.collector_account.as_deref(),
                &args.source,
                args.reserve_percent,
                StdDuration::from_secs(args.valid_seconds),
            )?),
            QuotaCommand::RecordUsage(args) => print_json(&garnish.record_quota_usage_sample(
                &args.evidence_id,
                &args.adapter,
                &args.provider,
                &args.account,
                &args.surface,
                args.estimated_seconds,
                args.consumed_percent,
                &args.source,
                &args.confidence,
                parse_optional_time(args.observed_at.as_deref())?.unwrap_or_else(Utc::now),
            )?),
            QuotaCommand::Forecast(args) => print_json(&garnish.usage_forecast(
                &args.adapter,
                &args.provider,
                &args.account,
                args.estimated_seconds,
                args.uncertainty_percent,
            )?),
            QuotaCommand::Samples { limit } => print_json(&garnish.quota_usage_samples(limit)?),
            QuotaCommand::Attempts => print_json(&garnish.quota_collection_attempts()?),
            QuotaCommand::Reservations => print_json(&garnish.quota_reservations()?),
            QuotaCommand::Status => print_json(&garnish.quota()?),
        },
        Command::Api { command } => match command {
            ApiCommand::BudgetSet(args) => {
                print_json(&garnish.configure_api_budget(&NewApiBudget {
                    project_id: args.project,
                    provider: args.provider,
                    account: args.account,
                    enabled: args.enabled,
                    secret_reference: args.secret_reference,
                    currency: args.currency,
                    currency_limit_micros: args.currency_limit_micros,
                    token_limit: args.token_limit,
                    request_limit: args.request_limit,
                    period_start: parse_required_time(&args.period_start)?,
                    period_end: parse_required_time(&args.period_end)?,
                    allowed_models: args.allowed_models,
                    allowed_tools: args.allowed_tools,
                    allowed_roles: args.allowed_roles,
                    max_output_tokens: args.max_output_tokens,
                    max_retries: args.max_retries,
                    max_concurrent_requests: args.max_concurrent_requests,
                    reason: args.reason,
                })?)
            }
            ApiCommand::BudgetStatus { project } => {
                print_json(&garnish.api_budgets(project.as_deref())?)
            }
            ApiCommand::Reservations { project } => {
                print_json(&garnish.api_reservations(project.as_deref())?)
            }
            ApiCommand::Spend { project } => print_json(&garnish.api_spend(project.as_deref())?),
        },
        Command::Schedule { command } => match command {
            ScheduleCommand::Configure {
                slug,
                timezone,
                weekly_pattern,
            } => print_json(&garnish.configure_calendar(&slug, &timezone, &weekly_pattern)?),
            ScheduleCommand::Assign { project, calendar } => {
                print_json(&garnish.assign_project_calendar(&project, &calendar)?)
            }
            ScheduleCommand::Exception {
                calendar,
                date,
                kind,
                reason,
            } => {
                let date = chrono::NaiveDate::parse_from_str(&date, "%Y-%m-%d")
                    .context("--date must be YYYY-MM-DD")?;
                let kind = DayKind::from_str(&kind)?;
                print_json(&garnish.set_calendar_exception(&calendar, date, kind, &reason)?)
            }
            ScheduleCommand::Evaluate { task, at } => {
                let at = parse_optional_time(at.as_deref())?.unwrap_or_else(Utc::now);
                print_json(&garnish.evaluate_task_schedule_at(&task, at)?)
            }
            ScheduleCommand::Preview {
                adapter,
                provider,
                account,
                at,
            } => {
                let at = parse_optional_time(at.as_deref())?.unwrap_or_else(Utc::now);
                print_json(&garnish.scheduler_preview_at(&adapter, &provider, &account, at)?)
            }
        },
        Command::Scheduler { command } => match command {
            SchedulerCommand::Daemon(args) => {
                SCHEDULER_SHUTDOWN.store(false, Ordering::SeqCst);
                install_scheduler_signal_handlers()?;
                let route_candidates = parse_route_targets(&args.candidates)?;
                let config = SchedulerDaemonConfig {
                    instance_id: args.instance,
                    hostname: args.hostname,
                    adapter: args.adapter,
                    provider: args.provider,
                    account: args.account,
                    route_candidates,
                    max_active_claims: args.max_active,
                    max_active_per_adapter: args.max_active_per_adapter,
                    max_active_per_account: args.max_active_per_account,
                    poll_interval: StdDuration::from_millis(args.poll_milliseconds),
                    leader_ttl: StdDuration::from_secs(args.leader_ttl_seconds),
                    claim_ttl: StdDuration::from_secs(args.claim_ttl_seconds),
                    max_ticks: args.max_ticks,
                    execute_fake_claims: args.execute_fake,
                };
                print_json(&garnish.run_scheduler_daemon(&config, &SCHEDULER_SHUTDOWN)?)
            }
            SchedulerCommand::Register { instance, hostname } => {
                let now = Utc::now();
                garnish.register_scheduler(&instance, &hostname, std::process::id(), now)?;
                print_json(&json!({
                    "instance_id": instance,
                    "hostname": hostname,
                    "process_id": std::process::id(),
                    "registered_at": now,
                }))
            }
            SchedulerCommand::AcquireLeader {
                instance,
                ttl_seconds,
            } => print_json(&garnish.acquire_scheduler_leader(
                &instance,
                Utc::now(),
                std::time::Duration::from_secs(ttl_seconds),
            )?),
            SchedulerCommand::Heartbeat {
                instance,
                generation,
                ttl_seconds,
            } => print_json(&garnish.heartbeat_scheduler_leader(
                &instance,
                generation,
                Utc::now(),
                std::time::Duration::from_secs(ttl_seconds),
            )?),
            SchedulerCommand::Tick {
                instance,
                generation,
                adapter,
                provider,
                account,
                candidates,
                max_active,
                max_active_per_adapter,
                max_active_per_account,
                claim_ttl_seconds,
                at,
            } => {
                let at = parse_optional_time(at.as_deref())?.unwrap_or_else(Utc::now);
                let mut route_targets = parse_route_targets(&candidates)?;
                if route_targets.is_empty() {
                    route_targets.push(RouteTarget {
                        adapter,
                        provider,
                        account,
                    });
                }
                print_json(&garnish.scheduler_tick_candidates_with_limits_at(
                    &instance,
                    generation,
                    &route_targets,
                    at,
                    max_active,
                    max_active_per_adapter,
                    max_active_per_account,
                    std::time::Duration::from_secs(claim_ttl_seconds),
                )?)
            }
            SchedulerCommand::Recover { at } => {
                let at = parse_optional_time(at.as_deref())?.unwrap_or_else(Utc::now);
                print_json(&json!({"recovered_task_ids": garnish.recover_scheduler(at)?}))
            }
            SchedulerCommand::Stop { instance } => print_json(&json!({
                "instance_id": instance,
                "status": "stopped",
                "released_task_ids": garnish.stop_scheduler(&instance, Utc::now())?,
            })),
            SchedulerCommand::Wakes => print_json(&garnish.scheduler_wakes()?),
        },
        Command::Runtime { command } => match command {
            RuntimeCommand::Runs { task } => print_json(&garnish.run_records(&task)?),
            RuntimeCommand::Checkpoint {
                run,
                provider,
                account,
                at,
            } => {
                let at = parse_optional_time(at.as_deref())?.unwrap_or_else(Utc::now);
                print_json(&garnish.checkpoint_run_at(&run, &provider, &account, at)?)
            }
            RuntimeCommand::Cancel { run, reason } => print_json(&json!({
                "run_id": run,
                "cancellation_requested": garnish.request_run_cancellation(&run, &reason)?,
                "reason": reason,
            })),
            RuntimeCommand::RetryState { task } => print_json(&garnish.retry_state(&task)?),
            RuntimeCommand::RetryLimit { task, limit } => {
                print_json(&garnish.set_retry_limit(&task, limit)?)
            }
            RuntimeCommand::Circuits => print_json(&garnish.adapter_circuits()?),
        },
        Command::Ops { command } => match command {
            OpsCommand::Status => print_json(&garnish.operational_status()?),
            OpsCommand::Pause { reason } => print_json(&garnish.pause_new_work(&reason)?),
            OpsCommand::Resume { reason } => print_json(&garnish.resume_operations(&reason)?),
            OpsCommand::EmergencyStop { reason } => print_json(&garnish.emergency_stop(&reason)?),
            OpsCommand::Diagnostics => print_json(&garnish.diagnostics()?),
            OpsCommand::Backup { output } => print_json(&garnish.create_backup(output.as_deref())?),
        },
        Command::Notification { command } => match command {
            NotificationCommand::List { all, limit } => {
                print_json(&garnish.local_notifications(all, limit)?)
            }
            NotificationCommand::Acknowledge { id } => {
                print_json(&garnish.acknowledge_notification(&id)?)
            }
        },
        Command::Agent { command } => match command {
            AgentCommand::Probe => print_json(&garnish.doctor().probes),
            AgentCommand::Refresh { valid_seconds } => print_json(
                &garnish
                    .refresh_agent_capabilities(std::time::Duration::from_secs(valid_seconds))?,
            ),
            AgentCommand::Status { at } => {
                let at = parse_optional_time(at.as_deref())?.unwrap_or_else(Utc::now);
                print_json(&garnish.agent_capability_status_at(at)?)
            }
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
        Command::Ui { command } => match command {
            UiCommand::Serve(args) => {
                serve_ui(
                    &garnish,
                    &UiServerConfig {
                        port: args.port,
                        max_requests: args.max_requests,
                    },
                    |ready| {
                        print_json(&json!({
                            "ok": true,
                            "mode": "read_only",
                            "listen": ready.listen,
                            "url": ready.url,
                        }))?;
                        std::io::stdout().flush()?;
                        Ok(())
                    },
                )?;
                Ok(())
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

fn parse_required_time(value: &str) -> Result<DateTime<Utc>> {
    DateTime::from_str(value).with_context(|| format!("invalid RFC3339 timestamp: {value}"))
}

fn parse_route_targets(values: &[String]) -> Result<Vec<RouteTarget>> {
    values
        .iter()
        .map(|value| {
            let mut parts = value.split(':');
            let adapter = parts.next().unwrap_or_default();
            let provider = parts.next().unwrap_or_default();
            let account = parts.next().unwrap_or_default();
            if adapter.is_empty()
                || provider.is_empty()
                || account.is_empty()
                || parts.next().is_some()
                || [adapter, provider, account]
                    .iter()
                    .any(|part| part.chars().any(char::is_whitespace))
            {
                bail!(
                    "invalid route candidate {value:?}; expected ADAPTER:PROVIDER:ACCOUNT without whitespace"
                );
            }
            Ok(RouteTarget {
                adapter: adapter.into(),
                provider: provider.into(),
                account: account.into(),
            })
        })
        .collect()
}

fn print_json(value: &impl Serialize) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

#[cfg(unix)]
extern "C" fn scheduler_signal_handler(_signal: libc::c_int) {
    SCHEDULER_SHUTDOWN.store(true, Ordering::SeqCst);
}

#[cfg(unix)]
fn install_scheduler_signal_handlers() -> Result<()> {
    // SAFETY: the handler only performs an atomic store, which is async-signal-safe.
    unsafe {
        if libc::signal(
            libc::SIGINT,
            scheduler_signal_handler as *const () as libc::sighandler_t,
        ) == libc::SIG_ERR
            || libc::signal(
                libc::SIGTERM,
                scheduler_signal_handler as *const () as libc::sighandler_t,
            ) == libc::SIG_ERR
        {
            bail!("failed to install scheduler shutdown signal handlers");
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn install_scheduler_signal_handlers() -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_candidate_parser_is_exact_and_rejects_malformed_or_extra_fields() {
        let parsed =
            parse_route_targets(&["codex:openai:primary".into(), "claude:anthropic:max".into()])
                .unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].adapter, "codex");
        assert_eq!(parsed[1].account, "max");
        for invalid in [
            "codex:openai",
            "codex:openai:primary:extra",
            "codex:open ai:primary",
            "::",
        ] {
            assert!(parse_route_targets(&[invalid.into()]).is_err());
        }
    }
}
