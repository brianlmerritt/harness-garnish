use crate::{
    Garnish,
    domain::{
        AgentCapabilityStatus, ApprovalRequest, LocalNotification, Project, QuotaCollectionAttempt,
        QuotaSurface, SchedulerWake, Task, TaskStatus,
    },
};
use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fmt::Write as _,
    io::{Read, Write},
    net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream},
    time::Duration,
};

const MAX_REQUEST_HEAD_BYTES: usize = 32 * 1024;
const SESSION_COOKIE: &str = "garnish_session";

#[derive(Debug, Clone)]
pub struct UiServerConfig {
    pub port: u16,
    pub max_requests: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiServerReady {
    pub listen: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiServerSummary {
    pub listen: String,
    pub requests_served: usize,
    pub stopped_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperatorSnapshot {
    pub evaluated_at: DateTime<Utc>,
    pub operational_status: serde_json::Value,
    pub projects: Vec<Project>,
    pub tasks: Vec<Task>,
    pub agents: Vec<AgentCapabilityStatus>,
    pub quotas: Vec<QuotaSurface>,
    pub approvals: Vec<ApprovalRequest>,
    pub notifications: Vec<LocalNotification>,
    pub scheduler_wakes: Vec<SchedulerWake>,
    pub quota_attempts: Vec<QuotaCollectionAttempt>,
}

pub fn operator_snapshot(garnish: &Garnish) -> Result<OperatorSnapshot> {
    let mut quota_attempts = garnish.quota_collection_attempts()?;
    quota_attempts.truncate(40);
    let mut scheduler_wakes = garnish.scheduler_wakes()?;
    scheduler_wakes.truncate(100);
    Ok(OperatorSnapshot {
        evaluated_at: Utc::now(),
        operational_status: garnish.operational_status()?,
        projects: garnish.projects()?,
        tasks: garnish.tasks(None)?,
        agents: garnish.agent_capability_status()?,
        quotas: garnish.quota()?,
        approvals: garnish.approvals(100)?,
        notifications: garnish.local_notifications(true, 40)?,
        scheduler_wakes,
        quota_attempts,
    })
}

pub fn serve_ui(
    garnish: &Garnish,
    config: &UiServerConfig,
    on_ready: impl FnOnce(&UiServerReady) -> Result<()>,
) -> Result<UiServerSummary> {
    if config.max_requests == Some(0) {
        bail!("UI max requests must be greater than zero when specified");
    }
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, config.port))
        .with_context(|| format!("binding Garnish UI to 127.0.0.1:{}", config.port))?;
    let address = listener.local_addr()?;
    if !address.ip().is_loopback() {
        bail!("Garnish UI must bind to a loopback address");
    }
    let token = generate_token()?;
    let listen = address.to_string();
    let ready = UiServerReady {
        listen: listen.clone(),
        url: format!("http://{listen}/?token={token}"),
    };
    on_ready(&ready)?;

    let mut requests_served = 0;
    for connection in listener.incoming() {
        let mut stream = connection.context("accepting Garnish UI connection")?;
        requests_served += 1;
        if let Err(error) = handle_connection(&mut stream, garnish, address, &token) {
            let _ = write_response(
                &mut stream,
                Response::html(
                    400,
                    "Bad Request",
                    &render_message_page(
                        "Request rejected",
                        "The local UI could not safely process this request.",
                    ),
                ),
                false,
            );
            eprintln!("Garnish UI request rejected: {error:#}");
        }
        if config
            .max_requests
            .is_some_and(|maximum| requests_served >= maximum)
        {
            break;
        }
    }
    Ok(UiServerSummary {
        listen,
        requests_served,
        stopped_at: Utc::now(),
    })
}

fn generate_token() -> Result<String> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).context("generating local UI authentication token")?;
    Ok(hex::encode(bytes))
}

#[derive(Debug)]
struct Request {
    method: String,
    target: String,
    headers: BTreeMap<String, String>,
}

fn handle_connection(
    stream: &mut TcpStream,
    garnish: &Garnish,
    address: SocketAddr,
    token: &str,
) -> Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    stream.set_nodelay(true)?;
    let request = read_request(stream)?;
    let head_only = request.method == "HEAD";
    if request.method != "GET" && !head_only {
        return write_response(
            stream,
            Response::text(405, "Method Not Allowed", "method not allowed")
                .header("Allow", "GET, HEAD"),
            false,
        );
    }
    let expected_host = address.to_string();
    if request.headers.get("host") != Some(&expected_host) {
        return write_response(
            stream,
            Response::text(421, "Misdirected Request", "invalid host"),
            head_only,
        );
    }
    let (path, query) = split_target(&request.target)?;
    if let Some(candidate) = query_token(query)
        && constant_time_eq(candidate.as_bytes(), token.as_bytes())
    {
        return write_response(
            stream,
            Response::empty(303, "See Other")
                .header("Location", path)
                .header(
                    "Set-Cookie",
                    &format!(
                        "{SESSION_COOKIE}={token}; HttpOnly; SameSite=Strict; Path=/; Max-Age=28800"
                    ),
                ),
            head_only,
        );
    }
    if !authorized(&request.headers, token) {
        let response = if path.starts_with("/api/") {
            Response::json(
                401,
                "Unauthorized",
                br#"{"error":"authentication required","ok":false}"#.to_vec(),
            )
        } else {
            Response::html(
                401,
                "Unauthorized",
                &render_message_page(
                    "Authentication required",
                    "Open the one-time URL printed by `garnish ui serve`.",
                ),
            )
        };
        return write_response(stream, response, head_only);
    }

    let response = match path {
        "/healthz" => Response::json(
            200,
            "OK",
            serde_json::to_vec(&serde_json::json!({
                "ok": true,
                "mode": "read_only",
                "evaluated_at": Utc::now(),
            }))?,
        ),
        "/api/v1/snapshot" => {
            Response::json(200, "OK", serde_json::to_vec(&operator_snapshot(garnish)?)?)
        }
        "/" | "/projects" | "/queue" | "/agents" | "/approvals" | "/activity" | "/settings" => {
            let snapshot = operator_snapshot(garnish)?;
            Response::html(
                200,
                "OK",
                &render_page(path, &snapshot, garnish.data_dir(), &expected_host),
            )
        }
        _ => Response::html(
            404,
            "Not Found",
            &render_message_page("Not found", "That local Garnish page does not exist."),
        ),
    };
    write_response(stream, response, head_only)
}

fn read_request(stream: &mut TcpStream) -> Result<Request> {
    let mut bytes = Vec::with_capacity(2048);
    let mut buffer = [0_u8; 2048];
    loop {
        let count = stream.read(&mut buffer)?;
        if count == 0 {
            bail!("connection closed before HTTP headers completed");
        }
        bytes.extend_from_slice(&buffer[..count]);
        if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
        if bytes.len() > MAX_REQUEST_HEAD_BYTES {
            bail!("HTTP request headers exceed safety limit");
        }
    }
    if bytes.len() > MAX_REQUEST_HEAD_BYTES {
        bail!("HTTP request headers exceed safety limit");
    }
    let end = bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .context("missing HTTP header terminator")?;
    let head = std::str::from_utf8(&bytes[..end]).context("HTTP headers are not UTF-8")?;
    let mut lines = head.split("\r\n");
    let request_line = lines.next().context("missing HTTP request line")?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().context("missing HTTP method")?;
    let target = parts.next().context("missing HTTP target")?;
    let version = parts.next().context("missing HTTP version")?;
    if parts.next().is_some() || version != "HTTP/1.1" {
        bail!("unsupported HTTP request line");
    }
    if !method.bytes().all(|byte| byte.is_ascii_uppercase())
        || target.bytes().any(|byte| byte.is_ascii_control())
    {
        bail!("invalid HTTP request line");
    }
    let mut headers = BTreeMap::new();
    for line in lines {
        let (name, value) = line.split_once(':').context("malformed HTTP header")?;
        if name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
            bail!("invalid HTTP header name");
        }
        let name = name.to_ascii_lowercase();
        if headers.insert(name, value.trim().to_owned()).is_some() {
            bail!("duplicate HTTP header");
        }
    }
    if headers
        .get("content-length")
        .is_some_and(|length| length != "0")
    {
        bail!("request bodies are not accepted");
    }
    Ok(Request {
        method: method.to_owned(),
        target: target.to_owned(),
        headers,
    })
}

fn split_target(target: &str) -> Result<(&str, Option<&str>)> {
    let (path, query) = target
        .split_once('?')
        .map_or((target, None), |(path, query)| (path, Some(query)));
    if !path.starts_with('/') || path.contains("//") || path.contains("..") || path.contains('#') {
        bail!("invalid HTTP target");
    }
    Ok((path, query))
}

fn query_token(query: Option<&str>) -> Option<&str> {
    query?.split('&').find_map(|pair| {
        let (name, value) = pair.split_once('=')?;
        (name == "token").then_some(value)
    })
}

fn authorized(headers: &BTreeMap<String, String>, token: &str) -> bool {
    let bearer = headers
        .get("authorization")
        .and_then(|value| value.strip_prefix("Bearer "));
    let cookie = headers.get("cookie").and_then(|cookies| {
        cookies.split(';').find_map(|cookie| {
            let (name, value) = cookie.trim().split_once('=')?;
            (name == SESSION_COOKIE).then_some(value)
        })
    });
    bearer
        .or(cookie)
        .is_some_and(|candidate| constant_time_eq(candidate.as_bytes(), token.as_bytes()))
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

struct Response {
    status: u16,
    reason: &'static str,
    content_type: &'static str,
    body: Vec<u8>,
    headers: Vec<(String, String)>,
}

impl Response {
    fn empty(status: u16, reason: &'static str) -> Self {
        Self {
            status,
            reason,
            content_type: "text/plain; charset=utf-8",
            body: Vec::new(),
            headers: Vec::new(),
        }
    }

    fn text(status: u16, reason: &'static str, body: &str) -> Self {
        Self {
            status,
            reason,
            content_type: "text/plain; charset=utf-8",
            body: body.as_bytes().to_vec(),
            headers: Vec::new(),
        }
    }

    fn html(status: u16, reason: &'static str, body: &str) -> Self {
        Self {
            status,
            reason,
            content_type: "text/html; charset=utf-8",
            body: body.as_bytes().to_vec(),
            headers: Vec::new(),
        }
    }

    fn json(status: u16, reason: &'static str, body: Vec<u8>) -> Self {
        Self {
            status,
            reason,
            content_type: "application/json; charset=utf-8",
            body,
            headers: Vec::new(),
        }
    }

    fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }
}

fn write_response(stream: &mut TcpStream, response: Response, head_only: bool) -> Result<()> {
    write!(
        stream,
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\nContent-Security-Policy: default-src 'none'; style-src 'unsafe-inline'; img-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'\r\nReferrer-Policy: no-referrer\r\nX-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\n",
        response.status,
        response.reason,
        response.content_type,
        response.body.len()
    )?;
    for (name, value) in response.headers {
        if name.contains(['\r', '\n']) || value.contains(['\r', '\n']) {
            bail!("invalid HTTP response header");
        }
        write!(stream, "{name}: {value}\r\n")?;
    }
    write!(stream, "\r\n")?;
    if !head_only {
        stream.write_all(&response.body)?;
    }
    stream.flush()?;
    Ok(())
}

fn render_page(
    path: &str,
    snapshot: &OperatorSnapshot,
    data_dir: &std::path::Path,
    listen: &str,
) -> String {
    let (title, eyebrow, content) = match path {
        "/projects" => (
            "Projects",
            "Topology and policy boundaries",
            render_projects(snapshot),
        ),
        "/queue" => ("Queue", "What runs next—and why", render_queue(snapshot)),
        "/agents" => (
            "Agents & quotas",
            "Capability, health, and headroom",
            render_agents(snapshot),
        ),
        "/approvals" => (
            "Approvals",
            "Explicit human decisions",
            render_approvals(snapshot),
        ),
        "/activity" => (
            "Activity",
            "Bounded operational evidence",
            render_activity(snapshot),
        ),
        "/settings" => (
            "Settings",
            "Local service and safety posture",
            render_settings(snapshot, data_dir, listen),
        ),
        _ => (
            "Overview",
            "Your local agent control plane",
            render_overview(snapshot),
        ),
    };
    format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>{title} · Harness Garnish</title><style>{}</style></head><body><div class="shell"><aside><a class="brand" href="/"><span class="brand-mark">G</span><span>Harness<br><strong>Garnish</strong></span></a><nav>{}</nav><div class="side-note"><span class="pulse"></span> Local &amp; read-only<br><small>Authenticated loopback</small></div></aside><main><header><div><p class="eyebrow">{eyebrow}</p><h1>{title}</h1></div><div class="asof">Updated<br><strong>{}</strong></div></header>{content}<footer>Harness Garnish · canonical state remains in SQLite · interface mutations are intentionally disabled in this first slice</footer></main></div></body></html>"#,
        styles(),
        render_nav(path),
        escape_html(&snapshot.evaluated_at.format("%H:%M:%S UTC").to_string()),
    )
}

fn render_nav(active: &str) -> String {
    [
        ("/", "Overview", "⌂"),
        ("/projects", "Projects", "◇"),
        ("/queue", "Queue", "≡"),
        ("/agents", "Agents & quotas", "◎"),
        ("/approvals", "Approvals", "✓"),
        ("/activity", "Activity", "↻"),
        ("/settings", "Settings", "⚙"),
    ]
    .into_iter()
    .map(|(href, label, icon)| {
        let current = if href == active {
            " aria-current=\"page\""
        } else {
            ""
        };
        format!(
            "<a href=\"{href}\"{current}><span>{icon}</span>{}</a>",
            escape_html(label)
        )
    })
    .collect()
}

fn render_overview(snapshot: &OperatorSnapshot) -> String {
    let ready = count_tasks(&snapshot.tasks, TaskStatus::Ready)
        + count_tasks(&snapshot.tasks, TaskStatus::Draft);
    let active = [
        TaskStatus::Leased,
        TaskStatus::Planning,
        TaskStatus::Running,
    ]
    .into_iter()
    .map(|status| count_tasks(&snapshot.tasks, status))
    .sum::<usize>();
    let attention = [
        TaskStatus::AwaitingApproval,
        TaskStatus::Paused,
        TaskStatus::Blocked,
        TaskStatus::Failed,
    ]
    .into_iter()
    .map(|status| count_tasks(&snapshot.tasks, status))
    .sum::<usize>();
    let control = &snapshot.operational_status["control"];
    let halted = control["emergency_stop"].as_bool().unwrap_or(false);
    let paused = control["pause_new_work"].as_bool().unwrap_or(false);
    let posture = if halted {
        ("Emergency stop", "danger")
    } else if paused {
        ("New work paused", "warn")
    } else {
        ("Ready", "good")
    };
    let mut html = format!(
        "<section class=\"hero\"><div><span class=\"badge {}\">{}</span><h2>Know what is happening.<br>Know why.</h2><p>One durable view of projects, agent capacity, quota boundaries, approvals, and the work Garnish is holding back.</p></div><div class=\"hero-orbit\"><div class=\"orbit-core\">{}<small>tasks tracked</small></div></div></section><section class=\"stats\"><article><span>Ready</span><strong>{ready}</strong><small>eligible or awaiting routing</small></article><article><span>Active</span><strong>{active}</strong><small>leased, planning, or running</small></article><article><span>Needs attention</span><strong>{attention}</strong><small>approval, pause, block, or failure</small></article><article><span>Quota surfaces</span><strong>{}</strong><small>independent provider limits</small></article></section>",
        posture.1,
        posture.0,
        snapshot.tasks.len(),
        snapshot.quotas.len()
    );
    html.push_str("<section class=\"split\"><div class=\"panel\"><div class=\"panel-head\"><div><p class=\"eyebrow\">Next up</p><h2>Queue at a glance</h2></div><a href=\"/queue\">Open queue →</a></div>");
    html.push_str(&render_task_rows(snapshot, 5));
    html.push_str("</div><div class=\"panel\"><div class=\"panel-head\"><div><p class=\"eyebrow\">Capacity</p><h2>Quota posture</h2></div><a href=\"/agents\">View agents →</a></div>");
    html.push_str(&render_quota_cards(snapshot, 4));
    html.push_str("</div></section>");
    html
}

fn render_projects(snapshot: &OperatorSnapshot) -> String {
    let mut html = String::from("<section class=\"cards\">");
    if snapshot.projects.is_empty() {
        html.push_str(&empty_state(
            "No projects yet",
            "Add a project with the CLI; it will appear here immediately.",
        ));
    }
    for project in &snapshot.projects {
        let tasks = snapshot
            .tasks
            .iter()
            .filter(|task| task.project_id == project.id)
            .collect::<Vec<_>>();
        let open = tasks
            .iter()
            .filter(|task| !task.status.is_terminal())
            .count();
        let state = if project.scheduler_paused {
            ("Paused", "warn")
        } else {
            ("Scheduling", "good")
        };
        let _ = write!(
            html,
            "<article class=\"project-card\"><div class=\"card-top\"><span class=\"badge {}\">{}</span><span>{} tasks</span></div><h2>{}</h2><p class=\"mono\">{}</p><div class=\"project-meta\"><span><strong>{open}</strong> open</span><span><strong>{}</strong> total</span></div>{}</article>",
            state.1,
            state.0,
            tasks.len(),
            escape_html(&project.title),
            escape_html(&project.root_path),
            tasks.len(),
            project
                .scheduler_pause_reason
                .as_ref()
                .map_or_else(String::new, |reason| format!(
                    "<p class=\"reason\">{}</p>",
                    escape_html(reason)
                )),
        );
    }
    html.push_str("</section>");
    html
}

fn render_queue(snapshot: &OperatorSnapshot) -> String {
    let mut html = String::from(
        "<section class=\"panel\"><div class=\"panel-head\"><div><p class=\"eyebrow\">Explainable scheduling</p><h2>All tasks</h2></div><span class=\"quiet\">Priority first</span></div>",
    );
    html.push_str(&render_task_rows(snapshot, usize::MAX));
    html.push_str("</section>");
    html
}

fn render_task_rows(snapshot: &OperatorSnapshot, limit: usize) -> String {
    if snapshot.tasks.is_empty() {
        return empty_state(
            "Nothing queued",
            "Tasks will appear here as projects are planned.",
        );
    }
    let project_names = snapshot
        .projects
        .iter()
        .map(|project| (project.id.as_str(), project.title.as_str()))
        .collect::<BTreeMap<_, _>>();
    let wakes = snapshot
        .scheduler_wakes
        .iter()
        .map(|wake| (wake.task_id.as_str(), wake))
        .collect::<BTreeMap<_, _>>();
    let mut tasks = snapshot.tasks.iter().collect::<Vec<_>>();
    tasks.sort_by(|left, right| {
        right
            .priority
            .cmp(&left.priority)
            .then_with(|| left.created_at.cmp(&right.created_at))
            .then_with(|| left.id.cmp(&right.id))
    });
    let mut html = String::from(
        "<div class=\"table-wrap\"><table><thead><tr><th>Task</th><th>Project</th><th>Status</th><th>Day</th><th>Priority</th><th>Why / next condition</th></tr></thead><tbody>",
    );
    for task in tasks.into_iter().take(limit) {
        let (status, class) = task_status(task.status);
        let project = project_names
            .get(task.project_id.as_str())
            .copied()
            .unwrap_or("Unknown project");
        let reason = task_reason(task, wakes.get(task.id.as_str()).copied());
        let _ = write!(
            html,
            "<tr><td><strong>{}</strong><small class=\"mono\">{}</small></td><td>{}</td><td><span class=\"badge {}\">{status}</span></td><td><span class=\"day\">{}</span></td><td>{}</td><td class=\"reason-cell\">{}</td></tr>",
            escape_html(&task.title),
            escape_html(&short_id(&task.id)),
            escape_html(project),
            class,
            task.day_affinity,
            task.priority,
            escape_html(&reason),
        );
    }
    html.push_str("</tbody></table></div>");
    html
}

fn render_agents(snapshot: &OperatorSnapshot) -> String {
    let mut html = String::from("<section class=\"cards agent-grid\">");
    for agent in &snapshot.agents {
        let class = status_class(&agent.health, &agent.freshness);
        let version = agent
            .probe
            .as_ref()
            .and_then(|probe| probe.version.as_deref())
            .unwrap_or("Not observed");
        let capabilities = agent.probe.as_ref().map_or_else(String::new, |probe| {
            probe
                .capabilities
                .iter()
                .take(5)
                .map(|capability| format!("<span>{}</span>", escape_html(capability)))
                .collect::<String>()
        });
        let _ = write!(
            html,
            "<article class=\"agent-card\"><div class=\"card-top\"><span class=\"badge {class}\">{} · {}</span><span class=\"agent-dot {class}\"></span></div><h2>{}</h2><p class=\"mono\">{}</p><div class=\"chips\">{capabilities}</div></article>",
            escape_html(&agent.health),
            escape_html(&agent.freshness),
            title_case(&agent.adapter),
            escape_html(version),
        );
    }
    html.push_str("</section><section class=\"panel section-gap\"><div class=\"panel-head\"><div><p class=\"eyebrow\">Independent limits</p><h2>Quota surfaces</h2></div><span class=\"quiet\">Reserve-aware</span></div>");
    html.push_str(&render_quota_cards(snapshot, usize::MAX));
    html.push_str("</section>");
    html
}

fn render_quota_cards(snapshot: &OperatorSnapshot, limit: usize) -> String {
    if snapshot.quotas.is_empty() {
        return empty_state(
            "No quota evidence",
            "Refresh a provider before Garnish allows quota-gated work.",
        );
    }
    let mut html = String::from("<div class=\"quota-list\">");
    for quota in snapshot.quotas.iter().take(limit) {
        let stale = quota
            .valid_until
            .is_some_and(|valid_until| valid_until <= snapshot.evaluated_at);
        let remaining = quota.effective_remaining_percent;
        let class = if stale || remaining.is_none() {
            "warn"
        } else if remaining.is_some_and(|value| value <= quota.reserve_percent) {
            "danger"
        } else {
            "good"
        };
        let label = if stale {
            "Stale".to_owned()
        } else {
            remaining.map_or_else(|| "Unknown".into(), |value| format!("{value:.0}% left"))
        };
        let width = remaining.unwrap_or(0.0).clamp(0.0, 100.0);
        let _ = write!(
            html,
            "<article class=\"quota-row\"><div><strong>{} · {}</strong><small>{} · reserve {:.0}%</small></div><div class=\"meter\"><i class=\"{class}\" style=\"width:{width:.1}%\"></i></div><span class=\"badge {class}\">{}</span></article>",
            title_case(&quota.provider),
            escape_html(&quota.surface.replace('_', " ")),
            escape_html(&quota.account),
            quota.reserve_percent,
            escape_html(&label),
        );
    }
    html.push_str("</div>");
    html
}

fn render_approvals(snapshot: &OperatorSnapshot) -> String {
    if snapshot.approvals.is_empty() {
        return format!(
            "<section class=\"panel\">{}</section>",
            empty_state(
                "No approval requests",
                "Actions requiring a human decision will be shown here with their exact effect class and expiry.",
            )
        );
    }
    let mut html = String::from(
        "<section class=\"panel\"><div class=\"panel-head\"><div><p class=\"eyebrow\">Decision queue</p><h2>Approval requests</h2></div><span class=\"badge neutral\">Read-only preview</span></div><div class=\"approval-list\">",
    );
    for approval in &snapshot.approvals {
        let expired = approval.expires_at <= snapshot.evaluated_at;
        let class = if approval.decision == "pending" && !expired {
            "warn"
        } else if approval.decision == "approved" {
            "good"
        } else {
            "neutral"
        };
        let state = if expired && approval.decision == "pending" {
            "expired"
        } else {
            &approval.decision
        };
        let action = truncate(&approval.action.to_string(), 180);
        let _ = write!(
            html,
            "<article class=\"approval\"><div><span class=\"badge {class}\">{}</span><span class=\"risk\">Effect class {}</span></div><h3>Task {}</h3><pre>{}</pre><small>Requested {} · expires {}</small></article>",
            escape_html(state),
            approval.effect_class,
            escape_html(&short_id(&approval.task_id)),
            escape_html(&action),
            escape_html(&format_time(approval.requested_at)),
            escape_html(&format_time(approval.expires_at)),
        );
    }
    html.push_str("</div></section>");
    html
}

fn render_activity(snapshot: &OperatorSnapshot) -> String {
    let mut html = String::from(
        "<section class=\"split\"><div class=\"panel\"><div class=\"panel-head\"><div><p class=\"eyebrow\">Provider evidence</p><h2>Quota collection</h2></div></div><div class=\"timeline\">",
    );
    if snapshot.quota_attempts.is_empty() {
        html.push_str(&empty_state(
            "No collection attempts",
            "Provider refresh attempts will be retained here.",
        ));
    }
    for attempt in &snapshot.quota_attempts {
        let class = if attempt.status == "succeeded" {
            "good"
        } else {
            "danger"
        };
        let _ = write!(
            html,
            "<article><i class=\"{class}\"></i><div><strong>{} · {}</strong><p>{}</p><small>{}</small></div></article>",
            title_case(&attempt.provider),
            escape_html(&attempt.status),
            escape_html(&attempt.detail),
            escape_html(&format_time(attempt.attempted_at)),
        );
    }
    html.push_str("</div></div><div class=\"panel\"><div class=\"panel-head\"><div><p class=\"eyebrow\">Local signals</p><h2>Notifications</h2></div></div><div class=\"timeline\">");
    if snapshot.notifications.is_empty() {
        html.push_str(&empty_state(
            "No notifications",
            "Operational notifications will appear here.",
        ));
    }
    for notification in &snapshot.notifications {
        let class = match notification.severity.as_str() {
            "critical" | "error" => "danger",
            "warning" => "warn",
            _ => "good",
        };
        let _ = write!(
            html,
            "<article><i class=\"{class}\"></i><div><strong>{}</strong><p>{}</p><small>{}{}</small></div></article>",
            escape_html(&notification.title),
            escape_html(&notification.body),
            escape_html(&format_time(notification.created_at)),
            if notification.acknowledged_at.is_some() {
                " · acknowledged"
            } else {
                ""
            },
        );
    }
    html.push_str("</div></div></section>");
    html
}

fn render_settings(
    snapshot: &OperatorSnapshot,
    data_dir: &std::path::Path,
    listen: &str,
) -> String {
    let control = &snapshot.operational_status["control"];
    let state = if control["emergency_stop"].as_bool().unwrap_or(false) {
        "Emergency stop"
    } else if control["pause_new_work"].as_bool().unwrap_or(false) {
        "New work paused"
    } else {
        "Normal operation"
    };
    format!(
        "<section class=\"settings-grid\"><article class=\"panel\"><p class=\"eyebrow\">Service</p><h2>Local interface</h2><dl><div><dt>Listen address</dt><dd class=\"mono\">{}</dd></div><div><dt>Authentication</dt><dd>Ephemeral capability cookie</dd></div><div><dt>Access</dt><dd>IPv4 loopback only</dd></div><div><dt>Mode</dt><dd>Read-only</dd></div></dl></article><article class=\"panel\"><p class=\"eyebrow\">Canonical state</p><h2>Storage</h2><dl><div><dt>Data directory</dt><dd class=\"mono\">{}</dd></div><div><dt>Projects</dt><dd>{}</dd></div><div><dt>Tasks</dt><dd>{}</dd></div><div><dt>Operational state</dt><dd>{state}</dd></div></dl></article><article class=\"panel wide\"><p class=\"eyebrow\">Safety boundary</p><h2>Why controls are not clickable yet</h2><p class=\"body-copy\">This first interface proves authenticated, explainable visibility over durable state. Pause, resume, approval, quota override, and emergency controls will be added only with CSRF protection, explicit confirmation, bounded inputs, and the same policy/evidence contracts as the CLI.</p></article></section>",
        escape_html(listen),
        escape_html(&data_dir.to_string_lossy()),
        snapshot.projects.len(),
        snapshot.tasks.len(),
    )
}

fn render_message_page(title: &str, message: &str) -> String {
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{}</title><style>{}</style></head><body class=\"message-page\"><main class=\"message-card\"><span class=\"brand-mark\">G</span><p class=\"eyebrow\">Harness Garnish</p><h1>{}</h1><p>{}</p></main></body></html>",
        escape_html(title),
        styles(),
        escape_html(title),
        escape_html(message)
    )
}

fn empty_state(title: &str, message: &str) -> String {
    format!(
        "<div class=\"empty\"><span>◇</span><strong>{}</strong><p>{}</p></div>",
        escape_html(title),
        escape_html(message)
    )
}

fn task_status(status: TaskStatus) -> (&'static str, &'static str) {
    match status {
        TaskStatus::Running | TaskStatus::Planning | TaskStatus::Leased => ("Active", "good"),
        TaskStatus::Ready | TaskStatus::Draft => ("Ready", "neutral"),
        TaskStatus::AwaitingApproval => ("Approval", "warn"),
        TaskStatus::Paused | TaskStatus::Blocked => ("Held", "warn"),
        TaskStatus::Failed | TaskStatus::Cancelled => ("Stopped", "danger"),
        TaskStatus::Completed | TaskStatus::Review | TaskStatus::Verifying => ("Review", "good"),
        TaskStatus::Superseded => ("Superseded", "neutral"),
    }
}

fn task_reason(task: &Task, wake: Option<&SchedulerWake>) -> String {
    if let Some(wake) = wake {
        return wake.wake_at.map_or_else(
            || wake.reason_code.clone(),
            |at| format!("{} · next check {}", wake.reason_code, format_time(at)),
        );
    }
    match task.status {
        TaskStatus::Draft => "Waiting for dependencies or scheduler evaluation".into(),
        TaskStatus::Ready => "Ready for the next eligible scheduler claim".into(),
        TaskStatus::AwaitingApproval => "Waiting for an explicit human decision".into(),
        TaskStatus::Running => "Running under checkpoint supervision".into(),
        TaskStatus::Review => "Implementation is ready for human review".into(),
        status => format!("Current lifecycle state: {status}"),
    }
}

fn status_class(health: &str, freshness: &str) -> &'static str {
    if freshness != "fresh" {
        "warn"
    } else if health == "healthy" {
        "good"
    } else {
        "danger"
    }
}

fn count_tasks(tasks: &[Task], status: TaskStatus) -> usize {
    tasks.iter().filter(|task| task.status == status).count()
}

fn short_id(value: &str) -> String {
    value.chars().take(10).collect()
}

fn truncate(value: &str, maximum: usize) -> String {
    let mut truncated = value.chars().take(maximum).collect::<String>();
    if value.chars().count() > maximum {
        truncated.push('…');
    }
    truncated
}

fn title_case(value: &str) -> String {
    let mut characters = value.chars();
    characters
        .next()
        .map(|first| first.to_uppercase().collect::<String>() + characters.as_str())
        .unwrap_or_default()
}

fn format_time(value: DateTime<Utc>) -> String {
    value.format("%Y-%m-%d %H:%M UTC").to_string()
}

fn escape_html(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            _ => escaped.push(character),
        }
    }
    escaped
}

fn styles() -> &'static str {
    r#"
:root{--ink:#17201b;--muted:#667069;--paper:#f4f3ed;--panel:#fffefa;--line:#dedfd6;--green:#1e6b4a;--lime:#b8d962;--amber:#c77d24;--red:#b7483f;--nav:#18251f;--shadow:0 18px 50px rgba(25,40,32,.08)}
*{box-sizing:border-box}html{background:var(--paper);color:var(--ink);font-family:Inter,ui-sans-serif,system-ui,-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif}body{margin:0}.shell{display:grid;grid-template-columns:240px minmax(0,1fr);min-height:100vh}aside{background:var(--nav);color:#eef2e9;padding:28px 20px;display:flex;flex-direction:column;position:sticky;top:0;height:100vh}.brand{display:flex;gap:12px;align-items:center;color:inherit;text-decoration:none;line-height:1.05;letter-spacing:.01em}.brand-mark{display:inline-grid;place-items:center;width:42px;height:42px;border-radius:13px;background:var(--lime);color:#17201b;font:800 22px/1 Georgia,serif;box-shadow:inset 0 0 0 1px rgba(0,0,0,.08)}nav{display:grid;gap:5px;margin-top:48px}nav a{display:flex;align-items:center;gap:12px;color:#aebbb3;text-decoration:none;padding:11px 12px;border-radius:10px;font-size:14px}nav a span{width:20px;text-align:center;color:#dbe5dd}nav a:hover,nav a[aria-current=page]{background:#26382f;color:white}nav a[aria-current=page]{box-shadow:inset 3px 0 var(--lime)}.side-note{margin-top:auto;border-top:1px solid #34463d;padding:18px 8px 0;color:#d9e2db;font-size:13px}.side-note small{color:#84978c;margin-left:14px}.pulse{display:inline-block;width:7px;height:7px;border-radius:50%;background:var(--lime);box-shadow:0 0 0 5px rgba(184,217,98,.12);margin-right:8px}main{padding:42px clamp(28px,5vw,72px);min-width:0}header{display:flex;align-items:flex-end;justify-content:space-between;margin-bottom:30px}h1,h2,h3,p{margin-top:0}h1{font:600 clamp(34px,4vw,52px)/1.02 Georgia,serif;letter-spacing:-.035em;margin-bottom:0}h2{font:600 24px/1.15 Georgia,serif;letter-spacing:-.02em}.eyebrow{text-transform:uppercase;letter-spacing:.14em;font-size:11px;font-weight:800;color:var(--green);margin-bottom:8px}.asof{text-align:right;color:var(--muted);font-size:11px}.asof strong{color:var(--ink);font-size:13px}.hero{background:var(--green);color:white;border-radius:24px;padding:clamp(28px,5vw,56px);display:flex;justify-content:space-between;align-items:center;overflow:hidden;position:relative;box-shadow:var(--shadow)}.hero:after{content:"";position:absolute;width:340px;height:340px;border-radius:50%;border:1px solid rgba(255,255,255,.13);right:-80px;top:-145px}.hero h2{font-size:clamp(34px,4vw,54px);max-width:650px;margin:22px 0 16px}.hero p{max-width:640px;color:#d7e7dc;line-height:1.65;margin-bottom:0}.hero-orbit{width:180px;height:180px;border:1px solid rgba(255,255,255,.24);border-radius:50%;display:grid;place-items:center;flex:none;margin-left:40px;position:relative}.hero-orbit:before{content:"";position:absolute;inset:20px;border:1px dashed rgba(255,255,255,.2);border-radius:50%}.orbit-core{width:108px;height:108px;border-radius:50%;background:var(--lime);color:var(--ink);display:grid;place-content:center;text-align:center;font:700 36px/1 Georgia,serif;z-index:1}.orbit-core small{font:600 10px/1.3 Inter,sans-serif;text-transform:uppercase;letter-spacing:.08em;margin-top:5px}.stats{display:grid;grid-template-columns:repeat(4,1fr);gap:14px;margin:18px 0}.stats article,.panel,.project-card,.agent-card{background:var(--panel);border:1px solid var(--line);border-radius:16px;box-shadow:0 4px 20px rgba(25,40,32,.035)}.stats article{padding:20px}.stats span,.stats small{display:block;color:var(--muted);font-size:12px}.stats strong{display:block;font:600 34px/1 Georgia,serif;margin:12px 0 8px}.split{display:grid;grid-template-columns:minmax(0,1.35fr) minmax(340px,.65fr);gap:18px}.panel{padding:24px;min-width:0}.panel-head,.card-top{display:flex;justify-content:space-between;align-items:flex-start;gap:16px}.panel-head{margin-bottom:18px}.panel-head h2{margin:0}.panel-head a{color:var(--green);font-size:13px;text-decoration:none;font-weight:700}.quiet{color:var(--muted);font-size:12px}.badge{display:inline-flex;align-items:center;border-radius:999px;padding:5px 9px;font-size:10px;font-weight:800;text-transform:uppercase;letter-spacing:.07em;white-space:nowrap}.badge.good{background:#deeedf;color:#245b3e}.badge.warn{background:#f5e7cc;color:#8d5919}.badge.danger{background:#f5dddd;color:#923a34}.badge.neutral{background:#e7e9e4;color:#58625c}.hero .badge.good{background:rgba(217,244,191,.17);color:#e5ffc4;border:1px solid rgba(229,255,196,.2)}.table-wrap{overflow:auto;margin:0 -8px}table{border-collapse:collapse;width:100%;font-size:13px}th{text-align:left;color:var(--muted);font-size:10px;text-transform:uppercase;letter-spacing:.08em;padding:10px 12px;border-bottom:1px solid var(--line)}td{padding:14px 12px;border-bottom:1px solid #ecece5;vertical-align:middle}tbody tr:last-child td{border-bottom:0}td strong{display:block}td small{display:block;color:var(--muted);margin-top:4px}.mono{font-family:"SFMono-Regular",Consolas,monospace;font-size:11px;word-break:break-all}.reason-cell{max-width:280px;color:#505a53}.day{display:inline-grid;place-items:center;width:28px;height:28px;border-radius:8px;background:#edf0e9;font-weight:800}.quota-list{display:grid;gap:12px}.quota-row{display:grid;grid-template-columns:minmax(150px,.8fr) minmax(90px,1fr) auto;gap:14px;align-items:center;padding:12px 0;border-bottom:1px solid #ecece5}.quota-row:last-child{border:0}.quota-row strong,.quota-row small{display:block}.quota-row small{color:var(--muted);margin-top:4px;font-size:11px}.meter{height:7px;border-radius:99px;background:#e8e9e2;overflow:hidden}.meter i{display:block;height:100%;border-radius:inherit;background:var(--green)}.meter i.warn{background:var(--amber)}.meter i.danger{background:var(--red)}.cards{display:grid;grid-template-columns:repeat(auto-fit,minmax(280px,1fr));gap:16px}.project-card,.agent-card{padding:24px}.project-card h2,.agent-card h2{margin:22px 0 8px}.project-card .card-top,.agent-card .card-top{color:var(--muted);font-size:12px}.project-meta{display:flex;gap:28px;border-top:1px solid var(--line);padding-top:18px;margin-top:22px;color:var(--muted);font-size:12px}.project-meta strong{font-size:18px;color:var(--ink);margin-right:4px}.reason{background:#f7ead5;color:#845c27;padding:10px 12px;border-radius:9px;font-size:12px;margin:18px 0 0}.agent-grid{grid-template-columns:repeat(3,1fr)}.agent-dot{width:9px;height:9px;border-radius:50%;background:var(--lime);box-shadow:0 0 0 6px rgba(184,217,98,.14)}.agent-dot.warn{background:var(--amber);box-shadow:0 0 0 6px rgba(199,125,36,.12)}.agent-dot.danger{background:var(--red);box-shadow:0 0 0 6px rgba(183,72,63,.12)}.chips{display:flex;gap:6px;flex-wrap:wrap;margin-top:18px}.chips span{padding:5px 8px;background:#eef0ea;border-radius:7px;font-size:10px;color:#59645d}.section-gap{margin-top:18px}.approval-list{display:grid;grid-template-columns:repeat(auto-fit,minmax(300px,1fr));gap:14px}.approval{border:1px solid var(--line);border-radius:13px;padding:18px;background:#fbfbf7}.approval .risk{font-size:11px;color:var(--muted);margin-left:10px}.approval h3{margin:16px 0 10px}.approval pre{white-space:pre-wrap;word-break:break-word;background:#eff1eb;border-radius:9px;padding:12px;font-size:11px;color:#3d4841}.approval small{color:var(--muted)}.timeline{display:grid}.timeline article{display:grid;grid-template-columns:12px 1fr;gap:12px;padding:13px 0;border-bottom:1px solid #ecece5}.timeline article:last-child{border:0}.timeline i{width:8px;height:8px;border-radius:50%;background:var(--green);margin-top:5px}.timeline i.warn{background:var(--amber)}.timeline i.danger{background:var(--red)}.timeline strong{font-size:13px}.timeline p{font-size:12px;color:#4e5952;line-height:1.45;margin:5px 0}.timeline small{font-size:10px;color:var(--muted)}.settings-grid{display:grid;grid-template-columns:1fr 1fr;gap:18px}.settings-grid .wide{grid-column:1/-1}.settings-grid dl{margin:20px 0 0}.settings-grid dl div{display:grid;grid-template-columns:140px 1fr;gap:12px;padding:12px 0;border-bottom:1px solid #ecece5}.settings-grid dt{color:var(--muted);font-size:12px}.settings-grid dd{margin:0;text-align:right;font-size:13px}.body-copy{color:#4f5b54;line-height:1.7;max-width:850px}.empty{text-align:center;padding:42px 24px;color:var(--muted)}.empty span{display:block;font-size:28px;color:#a8b0aa;margin-bottom:10px}.empty strong{display:block;color:var(--ink);margin-bottom:6px}.empty p{font-size:12px;margin:0 auto;max-width:420px;line-height:1.5}footer{margin-top:34px;padding-top:20px;border-top:1px solid var(--line);font-size:10px;color:#89908b}.message-page{min-height:100vh;display:grid;place-items:center;background:var(--paper)}.message-card{width:min(520px,calc(100% - 32px));background:var(--panel);border:1px solid var(--line);border-radius:20px;padding:40px;box-shadow:var(--shadow)}.message-card .brand-mark{margin-bottom:30px}.message-card h1{margin-bottom:16px}.message-card>p:last-child{color:var(--muted);line-height:1.6}
@media(max-width:980px){.shell{grid-template-columns:78px minmax(0,1fr)}aside{padding:24px 12px}.brand>span:last-child,nav a:not([aria-current=page]){font-size:0}.brand{justify-content:center}nav a{justify-content:center;padding:12px}nav a span{font-size:16px}.side-note{font-size:0;text-align:center}.side-note small{display:none}.hero-orbit{display:none}.stats{grid-template-columns:repeat(2,1fr)}.split{grid-template-columns:1fr}.agent-grid{grid-template-columns:1fr}.settings-grid{grid-template-columns:1fr}.settings-grid .wide{grid-column:auto}}
@media(max-width:640px){.shell{display:block}aside{position:static;height:auto;display:block;padding:14px}.brand{display:none}nav{display:flex;overflow:auto;margin:0;gap:4px}nav a,nav a:not([aria-current=page]){font-size:0;min-width:44px}nav a span{font-size:17px}.side-note{display:none}main{padding:26px 16px}header{align-items:flex-start}.asof{display:none}.hero{padding:28px 22px;border-radius:18px}.hero h2{font-size:34px}.stats{grid-template-columns:1fr 1fr}.stats article{padding:16px}.stats strong{font-size:28px}.panel{padding:17px}.quota-row{grid-template-columns:1fr auto}.quota-row .meter{grid-column:1/-1;grid-row:2}.settings-grid dl div{grid-template-columns:1fr}.settings-grid dd{text-align:left}.approval-list{grid-template-columns:1fr}}
"#
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{DayAffinity, NewTask};
    use std::{net::TcpStream, sync::mpsc, thread};
    use tempfile::tempdir;

    #[test]
    fn html_escaping_and_constant_time_token_check_are_exact() {
        assert_eq!(
            escape_html("<script x='y'>&\""),
            "&lt;script x=&#39;y&#39;&gt;&amp;&quot;"
        );
        assert!(constant_time_eq(b"same", b"same"));
        assert!(!constant_time_eq(b"same", b"diff"));
        assert!(!constant_time_eq(b"short", b"longer"));
    }

    #[test]
    fn rendered_queue_explains_a_durable_scheduler_wake() {
        let directory = tempdir().unwrap();
        let project_root = directory.path().join("project");
        std::fs::create_dir(&project_root).unwrap();
        let mut garnish = Garnish::open(directory.path().join("state")).unwrap();
        let project = garnish
            .add_project("fixture", "Fixture <Project>", &project_root)
            .unwrap();
        let task = garnish
            .add_task(&NewTask {
                project_id: project.id,
                title: "Inspect <unsafe> output".into(),
                goal: "Explain why it waits".into(),
                rationale: "fixture".into(),
                scope: vec![],
                non_scope: vec![],
                acceptance: vec!["reason visible".into()],
                verification_argv: vec!["true".into()],
                dependencies: vec![],
                priority: 7,
                risk_class: 0,
                estimated_seconds: 60,
                uncertainty_percent: 0,
                checkpoint_seconds: 60,
                day_affinity: DayAffinity::Both,
                deadline_at: None,
                required_capabilities: vec![],
                pinned_adapter: None,
                pinned_provider: None,
                pinned_account: None,
                fake_write_path: None,
                fake_write_content: None,
            })
            .unwrap();
        let now = Utc::now();
        garnish
            .register_scheduler("ui-fixture", "fixture", 1, now)
            .unwrap();
        let leader = garnish
            .acquire_scheduler_leader("ui-fixture", now, Duration::from_secs(60))
            .unwrap();
        let _ = garnish
            .scheduler_tick_at(
                "ui-fixture",
                leader.generation,
                "fake",
                "missing",
                "default",
                now,
                1,
                Duration::from_secs(60),
            )
            .unwrap();
        let snapshot = operator_snapshot(&garnish).unwrap();
        let html = render_page("/queue", &snapshot, garnish.data_dir(), "127.0.0.1:7467");
        assert!(html.contains("Inspect &lt;unsafe&gt; output"));
        assert!(html.contains("Fixture &lt;Project&gt;"));
        assert!(html.contains(&short_id(&task.id)));
        assert!(html.contains("quota.unavailable"));
        assert!(!html.contains("<unsafe>"));
    }

    #[test]
    fn loopback_server_requires_token_then_sets_strict_cookie_and_serves_api() {
        let directory = tempdir().unwrap();
        let state_dir = directory.path().join("state");
        let (ready_tx, ready_rx) = mpsc::channel();
        let server = thread::spawn(move || {
            let garnish = Garnish::open(state_dir).unwrap();
            serve_ui(
                &garnish,
                &UiServerConfig {
                    port: 0,
                    max_requests: Some(3),
                },
                |ready| {
                    ready_tx.send(ready.clone()).unwrap();
                    Ok(())
                },
            )
            .unwrap()
        });
        let ready = ready_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let token = ready.url.split("?token=").nth(1).unwrap();

        let unauthorized = request(&ready.listen, "/", &[]);
        assert!(unauthorized.starts_with("HTTP/1.1 401 Unauthorized\r\n"));

        let bootstrap = request(&ready.listen, &format!("/?token={token}"), &[]);
        assert!(bootstrap.starts_with("HTTP/1.1 303 See Other\r\n"));
        assert!(bootstrap.contains("HttpOnly; SameSite=Strict"));
        assert!(bootstrap.contains("Location: /\r\n"));

        let api = request(
            &ready.listen,
            "/api/v1/snapshot",
            &[("Authorization", &format!("Bearer {token}"))],
        );
        assert!(api.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(api.contains("\"operational_status\""));
        assert!(!api.contains(token));
        assert_eq!(server.join().unwrap().requests_served, 3);
    }

    fn request(address: &str, target: &str, headers: &[(&str, &str)]) -> String {
        let mut stream = TcpStream::connect(address).unwrap();
        write!(stream, "GET {target} HTTP/1.1\r\nHost: {address}\r\n").unwrap();
        for (name, value) in headers {
            write!(stream, "{name}: {value}\r\n").unwrap();
        }
        write!(stream, "Connection: close\r\n\r\n").unwrap();
        stream.shutdown(std::net::Shutdown::Write).unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    }
}
