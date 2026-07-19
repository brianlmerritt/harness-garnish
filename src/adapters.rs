use crate::process::{ProcessOutcome, ProcessSpec, supervise};
use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    env,
    ffi::OsString,
    fs,
    io::{Read, Write},
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeResult {
    pub adapter: String,
    pub executable: Option<String>,
    pub version: Option<String>,
    pub health: String,
    pub capabilities: Vec<String>,
    pub failure: Option<String>,
}

pub fn run_invocation(
    invocation: &Invocation,
    cancelled: Arc<AtomicBool>,
) -> Result<ProcessOutcome> {
    supervise(
        ProcessSpec {
            executable: &invocation.executable,
            argv: &invocation.argv,
            cwd: &invocation.cwd,
            environment: &invocation.environment,
            stdin: &invocation.stdin,
            timeout: invocation.timeout,
            termination_grace: Duration::from_secs(3),
            output_limit: invocation.output_limit,
        },
        cancelled,
    )
}

#[derive(Debug, Clone)]
pub struct Invocation {
    pub executable: PathBuf,
    pub argv: Vec<OsString>,
    pub cwd: PathBuf,
    pub environment: BTreeMap<String, String>,
    pub stdin: Vec<u8>,
    pub structured_protocol: Option<String>,
    pub timeout: Duration,
    pub output_limit: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentKind {
    Codex,
    Claude,
    Antigravity,
    Fake,
}

impl AgentKind {
    pub fn key(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
            Self::Antigravity => "antigravity",
            Self::Fake => "fake",
        }
    }

    pub fn probe(self) -> ProbeResult {
        if self == Self::Fake {
            return ProbeResult {
                adapter: "fake".into(),
                executable: None,
                version: Some("1".into()),
                health: "healthy".into(),
                capabilities: vec!["agent.structured_output=fake-jsonl@1".into()],
                failure: None,
            };
        }
        let command = match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
            Self::Antigravity => "agy",
            Self::Fake => unreachable!(),
        };
        let capabilities = match self {
            Self::Codex => vec![
                "agent.headless".into(),
                "agent.structured_output=jsonl".into(),
                "session.resume".into(),
                "agent.output_schema".into(),
            ],
            Self::Claude => vec![
                "agent.headless".into(),
                "agent.structured_output=stream-json".into(),
                "session.resume".into(),
                "agent.output_schema".into(),
                "agent.tool_policy".into(),
            ],
            Self::Antigravity => vec![
                "agent.headless=text".into(),
                "session.resume".into(),
                "agent.sandbox_flag".into(),
            ],
            Self::Fake => unreachable!(),
        };
        let mut result = probe_command(command, capabilities);
        if result.health == "healthy"
            && !result
                .version
                .as_deref()
                .is_some_and(|version| supported_agent_version(self, version))
        {
            result.health = "unsupported".into();
            result.failure = Some(format!(
                "installed {} version is outside the Phase 1 compatibility fixture range",
                self.key()
            ));
        }
        result
    }

    pub fn invocation(self, cwd: &Path, prompt: &str) -> Result<Invocation> {
        if self == Self::Fake {
            bail!("fake agent is executed internally");
        }
        let command = if self == Self::Antigravity {
            "agy"
        } else {
            self.key()
        };
        let executable = discover_executable(command)
            .ok_or_else(|| anyhow!("{} executable not found", self.key()))?;
        self.build_invocation(executable, cwd, prompt)
    }

    pub fn build_invocation(
        self,
        executable: PathBuf,
        cwd: &Path,
        prompt: &str,
    ) -> Result<Invocation> {
        if self == Self::Fake {
            bail!("fake agent is executed internally");
        }
        let mut argv = Vec::new();
        let mut stdin = Vec::new();
        let (structured_protocol, timeout) = match self {
            Self::Codex => {
                argv.extend([
                    OsString::from("exec"),
                    OsString::from("--json"),
                    OsString::from("--strict-config"),
                    OsString::from("--sandbox"),
                    OsString::from("workspace-write"),
                    OsString::from("--cd"),
                    cwd.as_os_str().to_owned(),
                    OsString::from("-"),
                ]);
                stdin.extend_from_slice(prompt.as_bytes());
                (Some("jsonl".into()), Duration::from_secs(45 * 60))
            }
            Self::Claude => {
                argv.extend([
                    OsString::from("--print"),
                    OsString::from("--output-format"),
                    OsString::from("stream-json"),
                    OsString::from("--permission-mode"),
                    OsString::from("dontAsk"),
                    OsString::from("--no-session-persistence"),
                    OsString::from(prompt),
                ]);
                (Some("stream-json".into()), Duration::from_secs(45 * 60))
            }
            Self::Antigravity => {
                argv.extend([
                    OsString::from("--print"),
                    OsString::from(prompt),
                    OsString::from("--print-timeout"),
                    OsString::from("45m"),
                    OsString::from("--mode"),
                    OsString::from("accept-edits"),
                ]);
                (None, Duration::from_secs(45 * 60))
            }
            Self::Fake => unreachable!(),
        };
        Ok(Invocation {
            executable,
            argv,
            cwd: cwd.to_path_buf(),
            environment: BTreeMap::new(),
            stdin,
            structured_protocol,
            timeout,
            output_limit: 2 * 1024 * 1024,
        })
    }
}

pub fn parse_structured_event(protocol: Option<&str>, line: &[u8]) -> Result<serde_json::Value> {
    match protocol {
        Some("jsonl" | "stream-json") => {
            let value: serde_json::Value =
                serde_json::from_slice(line).context("malformed structured agent event")?;
            if !value.is_object() {
                bail!("structured agent event must be a JSON object");
            }
            Ok(value)
        }
        None => Ok(serde_json::json!({
            "type": "text",
            "text": String::from_utf8_lossy(line),
        })),
        Some(other) => bail!("unsupported agent event protocol: {other}"),
    }
}

pub fn discover_executable(name: &str) -> Option<PathBuf> {
    if let Some(home) = directories::UserDirs::new().map(|dirs| dirs.home_dir().to_path_buf()) {
        let candidate = home.join(".local/bin").join(name);
        if is_executable_file(&candidate) {
            return candidate.canonicalize().ok().or(Some(candidate));
        }
    }
    if let Some(path) = env::var_os("PATH") {
        for directory in env::split_paths(&path) {
            let candidate = directory.join(name);
            if is_executable_file(&candidate) {
                return candidate.canonicalize().ok().or(Some(candidate));
            }
        }
    }
    if let Some(home) = directories::UserDirs::new().map(|dirs| dirs.home_dir().to_path_buf()) {
        let candidate = home.join("bin").join(name);
        if is_executable_file(&candidate) {
            return candidate.canonicalize().ok().or(Some(candidate));
        }
    }
    if name == "codex" {
        let bundled = PathBuf::from("/Applications/ChatGPT.app/Contents/Resources/codex");
        if is_executable_file(&bundled) {
            return bundled.canonicalize().ok().or(Some(bundled));
        }
    }
    None
}

#[cfg(unix)]
fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .map(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable_file(path: &Path) -> bool {
    path.is_file()
}

fn probe_command(command: &str, capabilities: Vec<String>) -> ProbeResult {
    let adapter = if command == "agy" {
        "antigravity"
    } else {
        command
    }
    .to_owned();
    let Some(executable) = discover_executable(command) else {
        return ProbeResult {
            adapter,
            executable: None,
            version: None,
            health: "missing".into(),
            capabilities,
            failure: Some(format!("{command} executable not found")),
        };
    };
    match Command::new(&executable)
        .arg("--version")
        .stdin(Stdio::null())
        .output()
    {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            ProbeResult {
                adapter,
                executable: Some(executable.to_string_lossy().into_owned()),
                version: Some(version),
                health: "healthy".into(),
                capabilities,
                failure: None,
            }
        }
        Ok(output) => ProbeResult {
            adapter,
            executable: Some(executable.to_string_lossy().into_owned()),
            version: None,
            health: "unhealthy".into(),
            capabilities,
            failure: Some(String::from_utf8_lossy(&output.stderr).trim().to_owned()),
        },
        Err(error) => ProbeResult {
            adapter,
            executable: Some(executable.to_string_lossy().into_owned()),
            version: None,
            health: "unhealthy".into(),
            capabilities,
            failure: Some(error.to_string()),
        },
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxAttestation {
    pub backend: String,
    pub secure_container: bool,
    pub image: String,
    pub writable_mounts: Vec<String>,
    pub network: String,
    pub user: String,
    pub container_socket_mounted: bool,
    pub host_home_mounted: bool,
    pub cpu_limit: String,
    pub memory_limit: String,
    pub pids_limit: u32,
    pub reasons: Vec<String>,
}

pub struct FakeSandbox;

impl FakeSandbox {
    pub fn attest(worktree: &Path) -> SandboxAttestation {
        SandboxAttestation {
            backend: "fake".into(),
            secure_container: true,
            image: "fake@sha256:deterministic".into(),
            writable_mounts: vec![worktree.to_string_lossy().into_owned()],
            network: "none".into(),
            user: "1000:1000".into(),
            container_socket_mounted: false,
            host_home_mounted: false,
            cpu_limit: "1".into(),
            memory_limit: "256m".into(),
            pids_limit: 64,
            reasons: vec![],
        }
    }
}

#[derive(Debug, Clone)]
pub struct DockerSpec {
    pub name: String,
    pub image: String,
    pub worktree: PathBuf,
    pub command: Vec<String>,
}

impl DockerSpec {
    pub fn create_argv(&self) -> Result<Vec<OsString>> {
        if !self.image.contains("@sha256:") {
            bail!("Docker image must be pinned by sha256 digest");
        }
        let worktree = self.worktree.canonicalize()?;
        let mut argv = vec![
            "create".into(),
            "--name".into(),
            self.name.clone().into(),
            "--label".into(),
            "harness-garnish.managed=true".into(),
            "--network".into(),
            "none".into(),
            "--read-only".into(),
            "--cap-drop".into(),
            "ALL".into(),
            "--security-opt".into(),
            "no-new-privileges".into(),
            "--pids-limit".into(),
            "256".into(),
            "--memory".into(),
            "2g".into(),
            "--cpus".into(),
            "2".into(),
            "--user".into(),
            "1000:1000".into(),
            "--workdir".into(),
            "/workspace".into(),
            "--mount".into(),
            format!("type=bind,src={},dst=/workspace", worktree.display()).into(),
            "--tmpfs".into(),
            "/tmp:rw,noexec,nosuid,nodev,size=256m".into(),
            self.image.clone().into(),
        ];
        argv.extend(self.command.iter().map(OsString::from));
        Ok(argv)
    }
}

#[derive(Debug, Clone)]
pub struct DockerBackend {
    executable: PathBuf,
}

impl DockerBackend {
    pub fn discover() -> Result<Self> {
        Ok(Self {
            executable: discover_executable("docker")
                .ok_or_else(|| anyhow!("docker executable not found"))?,
        })
    }

    pub fn create(&self, spec: &DockerSpec) -> Result<String> {
        let output = Command::new(&self.executable)
            .args(spec.create_argv()?)
            .stdin(Stdio::null())
            .output()
            .context("creating Docker sandbox")?;
        if !output.status.success() {
            bail!(
                "docker create failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(String::from_utf8(output.stdout)?.trim().into())
    }

    pub fn inspect(&self, spec: &DockerSpec) -> Result<SandboxAttestation> {
        let output = Command::new(&self.executable)
            .args(["inspect", &spec.name])
            .stdin(Stdio::null())
            .output()
            .context("inspecting Docker sandbox")?;
        if !output.status.success() {
            bail!(
                "docker inspect failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        let values: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout)?;
        let value = values
            .first()
            .context("docker inspect returned no container")?;
        attest_docker_inspect(spec, value)
    }

    pub fn start_attached(&self, name: &str) -> Result<std::process::Output> {
        Command::new(&self.executable)
            .args(["start", "--attach", name])
            .stdin(Stdio::null())
            .output()
            .context("starting Docker sandbox")
    }

    pub fn remove(&self, name: &str) -> Result<()> {
        let output = Command::new(&self.executable)
            .args(["rm", "--force", name])
            .stdin(Stdio::null())
            .output()
            .context("removing Docker sandbox")?;
        if !output.status.success() {
            bail!(
                "docker remove failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(())
    }
}

fn attest_docker_inspect(
    spec: &DockerSpec,
    value: &serde_json::Value,
) -> Result<SandboxAttestation> {
    let host = &value["HostConfig"];
    let config = &value["Config"];
    let mounts = value["Mounts"].as_array().cloned().unwrap_or_default();
    let expected_worktree = spec.worktree.canonicalize()?.to_string_lossy().into_owned();
    let bind_mounts: Vec<_> = mounts
        .iter()
        .filter(|mount| mount["Type"] == "bind")
        .collect();
    let writable_mounts: Vec<String> = bind_mounts
        .iter()
        .filter(|mount| mount["RW"] == true)
        .filter_map(|mount| mount["Source"].as_str().map(str::to_owned))
        .collect();
    let container_socket_mounted = bind_mounts.iter().any(|mount| {
        mount["Source"]
            .as_str()
            .is_some_and(|source| source.contains("docker.sock") || source.contains("podman.sock"))
    });
    let home = directories::UserDirs::new()
        .map(|dirs| dirs.home_dir().to_string_lossy().into_owned())
        .unwrap_or_default();
    let host_home_mounted = !home.is_empty()
        && bind_mounts
            .iter()
            .any(|mount| mount["Source"].as_str() == Some(home.as_str()));
    let cap_drop_all = host["CapDrop"]
        .as_array()
        .is_some_and(|values| values.iter().any(|value| value == "ALL"));
    let no_new_privileges = host["SecurityOpt"]
        .as_array()
        .is_some_and(|values| values.iter().any(|value| value == "no-new-privileges"));
    let pids_limit = host["PidsLimit"].as_u64().unwrap_or_default() as u32;
    let memory = host["Memory"].as_u64().unwrap_or_default();
    let nano_cpus = host["NanoCpus"].as_u64().unwrap_or_default();
    let expected_mount = bind_mounts.len() == 1
        && writable_mounts == vec![expected_worktree.clone()]
        && bind_mounts[0]["Destination"] == "/workspace";
    let checks = [
        (host["NetworkMode"] == "none", "network is not disabled"),
        (
            host["ReadonlyRootfs"] == true,
            "root filesystem is writable",
        ),
        (cap_drop_all, "capabilities were not fully dropped"),
        (no_new_privileges, "no-new-privileges is missing"),
        (pids_limit == 256, "PID limit differs from requested policy"),
        (
            memory == 2 * 1024 * 1024 * 1024,
            "memory limit differs from requested policy",
        ),
        (
            nano_cpus == 2_000_000_000,
            "CPU limit differs from requested policy",
        ),
        (
            config["User"] == "1000:1000",
            "container user is not 1000:1000",
        ),
        (
            expected_mount,
            "effective bind mounts differ from the sole worktree mount",
        ),
        (
            !container_socket_mounted,
            "a container runtime socket is mounted",
        ),
        (!host_home_mounted, "the host home directory is mounted"),
    ];
    let reasons: Vec<String> = checks
        .into_iter()
        .filter(|(passed, _)| !passed)
        .map(|(_, reason)| reason.to_owned())
        .collect();
    Ok(SandboxAttestation {
        backend: "docker".into(),
        secure_container: reasons.is_empty(),
        image: config["Image"].as_str().unwrap_or(&spec.image).into(),
        writable_mounts,
        network: host["NetworkMode"].as_str().unwrap_or("unknown").into(),
        user: config["User"].as_str().unwrap_or_default().into(),
        container_socket_mounted,
        host_home_mounted,
        cpu_limit: format!("{}", nano_cpus as f64 / 1_000_000_000.0),
        memory_limit: memory.to_string(),
        pids_limit,
        reasons,
    })
}

pub fn probe_docker() -> ProbeResult {
    let capabilities = vec![
        "sandbox.backend=docker".into(),
        "sandbox.network=none".into(),
        "sandbox.resource_limits".into(),
    ];
    let Some(executable) = discover_executable("docker") else {
        return ProbeResult {
            adapter: "docker".into(),
            executable: None,
            version: None,
            health: "missing".into(),
            capabilities,
            failure: Some("docker executable not found".into()),
        };
    };
    match Command::new(&executable)
        .args(["info", "--format", "{{.ServerVersion}}"])
        .stdin(Stdio::null())
        .output()
    {
        Ok(output) if output.status.success() => ProbeResult {
            adapter: "docker".into(),
            executable: Some(executable.to_string_lossy().into_owned()),
            version: Some(String::from_utf8_lossy(&output.stdout).trim().to_owned()),
            health: "healthy".into(),
            capabilities,
            failure: None,
        },
        Ok(output) => ProbeResult {
            adapter: "docker".into(),
            executable: Some(executable.to_string_lossy().into_owned()),
            version: None,
            health: "unhealthy".into(),
            capabilities,
            failure: Some(String::from_utf8_lossy(&output.stderr).trim().to_owned()),
        },
        Err(error) => ProbeResult {
            adapter: "docker".into(),
            executable: Some(executable.to_string_lossy().into_owned()),
            version: None,
            health: "unhealthy".into(),
            capabilities,
            failure: Some(error.to_string()),
        },
    }
}

pub fn probe_aoe() -> ProbeResult {
    let mut result = probe_command(
        "aoe",
        vec![
            "execution.session".into(),
            "execution.pty".into(),
            "execution.http-api".into(),
            "execution.acp".into(),
        ],
    );
    if result.health == "healthy" && !result.version.as_deref().is_some_and(supported_aoe_version) {
        result.health = "unsupported".into();
        result.failure = Some("supported AoE range is >=1.13.0,<2.0.0".into());
    }
    result
}

fn supported_aoe_version(output: &str) -> bool {
    numeric_version(output).is_some_and(|(major, minor, _)| major == 1 && minor >= 13)
}

fn supported_agent_version(agent: AgentKind, output: &str) -> bool {
    numeric_version(output).is_some_and(|(major, minor, _)| match agent {
        AgentKind::Codex => major == 0 && minor >= 144,
        AgentKind::Claude => major == 2,
        AgentKind::Antigravity => major == 1 && minor >= 1,
        AgentKind::Fake => true,
    })
}

fn numeric_version(output: &str) -> Option<(u64, u64, u64)> {
    output.split_whitespace().find_map(|token| {
        let token = token.trim_start_matches('v');
        let mut parts = token.split(|ch: char| !ch.is_ascii_digit() && ch != '.');
        let candidate = parts.next()?;
        let mut numbers = candidate.split('.');
        Some((
            numbers.next()?.parse().ok()?,
            numbers.next()?.parse().ok()?,
            numbers.next().unwrap_or("0").parse().ok()?,
        ))
    })
}

#[derive(Debug, Clone)]
pub struct AoeClient {
    address: SocketAddr,
    token: String,
}

impl AoeClient {
    pub fn loopback(port: u16, token: String) -> Result<Self> {
        if token.trim().is_empty() {
            bail!("AoE authentication token is required");
        }
        Ok(Self {
            address: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port),
            token,
        })
    }

    pub fn create_session(
        &self,
        path: &Path,
        tool: &str,
        title: &str,
    ) -> Result<serde_json::Value> {
        self.request(
            "POST",
            "/api/sessions",
            Some(&serde_json::json!({
                "path": path,
                "tool": tool,
                "title": title,
                "worktree_enabled": true,
                "create_new_branch": false,
            })),
        )
    }

    pub fn send(&self, session_id: &str, message: &str) -> Result<serde_json::Value> {
        validate_url_segment(session_id)?;
        self.request(
            "POST",
            &format!("/api/sessions/{session_id}/send"),
            Some(&serde_json::json!({"message": message})),
        )
    }

    pub fn output(&self, session_id: &str) -> Result<serde_json::Value> {
        validate_url_segment(session_id)?;
        self.request(
            "GET",
            &format!("/api/sessions/{session_id}/output?lines=200&format=text"),
            None,
        )
    }

    pub fn status(&self, session_id: &str) -> Result<serde_json::Value> {
        validate_url_segment(session_id)?;
        self.request("GET", &format!("/api/sessions/{session_id}"), None)
    }

    pub fn cancel(&self, session_id: &str) -> Result<serde_json::Value> {
        validate_url_segment(session_id)?;
        self.request("DELETE", &format!("/api/sessions/{session_id}"), None)
    }

    fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<&serde_json::Value>,
    ) -> Result<serde_json::Value> {
        let body = body
            .map(serde_json::to_vec)
            .transpose()?
            .unwrap_or_default();
        let mut stream = TcpStream::connect_timeout(&self.address, Duration::from_secs(2))
            .with_context(|| format!("connecting to AoE at {}", self.address))?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;
        write!(
            stream,
            "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            self.address.port(),
            self.token,
            body.len()
        )?;
        stream.write_all(&body)?;
        let mut response = Vec::new();
        stream.read_to_end(&mut response)?;
        let response = String::from_utf8(response)?;
        let (head, body) = response
            .split_once("\r\n\r\n")
            .ok_or_else(|| anyhow!("invalid AoE HTTP response"))?;
        let status = head
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|value| value.parse::<u16>().ok())
            .ok_or_else(|| anyhow!("invalid AoE HTTP status"))?;
        if !(200..300).contains(&status) {
            bail!("AoE HTTP {status}: {body}");
        }
        serde_json::from_str(body).context("parsing AoE JSON response")
    }
}

fn validate_url_segment(value: &str) -> Result<()> {
    if value.is_empty()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        bail!("invalid URL path segment");
    }
    Ok(())
}

pub fn safe_write(root: &Path, relative: &Path, content: &[u8]) -> Result<PathBuf> {
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        bail!("write path must be a normal relative path");
    }
    let target = root.join(relative);
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
        let canonical_parent = parent.canonicalize()?;
        let canonical_root = root.canonicalize()?;
        if !canonical_parent.starts_with(&canonical_root) {
            bail!("write path escapes worktree");
        }
    }
    if target
        .symlink_metadata()
        .is_ok_and(|metadata| metadata.file_type().is_symlink())
    {
        bail!("refusing to write through symlink");
    }
    fs::write(&target, content)?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{BufRead, BufReader},
        net::TcpListener,
        sync::{Arc, Mutex},
        thread,
    };
    use tempfile::tempdir;

    #[test]
    fn docker_args_are_hardened_and_do_not_mount_credentials() {
        let dir = tempdir().unwrap();
        let argv = DockerSpec {
            name: "garnish-test".into(),
            image: "example.invalid/image@sha256:abc".into(),
            worktree: dir.path().to_path_buf(),
            command: vec!["true".into()],
        }
        .create_argv()
        .unwrap();
        let text = argv
            .iter()
            .map(|v| v.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(text.contains("--network none"));
        assert!(text.contains("--cap-drop ALL"));
        assert!(text.contains("no-new-privileges"));
        assert!(!text.contains("docker.sock"));
        assert!(!text.contains(".ssh"));
        assert!(!text.contains(".claude"));
    }

    #[test]
    fn docker_requires_digest() {
        let dir = tempdir().unwrap();
        let spec = DockerSpec {
            name: "garnish-test".into(),
            image: "latest".into(),
            worktree: dir.path().to_path_buf(),
            command: vec![],
        };
        assert!(spec.create_argv().is_err());
    }

    #[test]
    fn safe_write_rejects_escape_and_symlink() {
        let dir = tempdir().unwrap();
        assert!(safe_write(dir.path(), Path::new("../escape"), b"x").is_err());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("outside", dir.path().join("link")).unwrap();
            assert!(safe_write(dir.path(), Path::new("link"), b"x").is_err());
        }
    }

    #[test]
    fn invocation_keeps_prompt_out_of_shell() {
        if let Some(_codex) = discover_executable("codex") {
            let invocation = AgentKind::Codex
                .invocation(Path::new("/tmp"), "$(touch /tmp/nope)")
                .unwrap();
            assert_eq!(invocation.argv.last().unwrap(), "-");
            assert!(
                !invocation
                    .argv
                    .iter()
                    .any(|arg| arg == "$(touch /tmp/nope)")
            );
        }
    }

    #[test]
    fn all_agent_invocations_use_argv_boundaries_and_expected_protocols() {
        let cwd = Path::new("/tmp");
        let prompt = "$(touch /tmp/not-created); newline\nvalue";
        let codex = AgentKind::Codex
            .build_invocation(PathBuf::from("/fake/codex"), cwd, prompt)
            .unwrap();
        assert_eq!(codex.stdin, prompt.as_bytes());
        assert_eq!(codex.structured_protocol.as_deref(), Some("jsonl"));
        assert!(!codex.argv.iter().any(|arg| arg == prompt));

        let claude = AgentKind::Claude
            .build_invocation(PathBuf::from("/fake/claude"), cwd, prompt)
            .unwrap();
        assert_eq!(claude.structured_protocol.as_deref(), Some("stream-json"));
        assert!(claude.argv.iter().any(|arg| arg == prompt));

        let antigravity = AgentKind::Antigravity
            .build_invocation(PathBuf::from("/fake/agy"), cwd, prompt)
            .unwrap();
        assert!(antigravity.argv.iter().any(|arg| arg == prompt));
        assert_eq!(antigravity.structured_protocol, None);
    }

    #[test]
    fn structured_parser_rejects_partial_and_schema_drift() {
        assert!(parse_structured_event(Some("jsonl"), br#"{"type":"done"}"#).is_ok());
        assert!(parse_structured_event(Some("jsonl"), br#"{"type": "#).is_err());
        assert!(parse_structured_event(Some("jsonl"), b"[]").is_err());
        assert!(parse_structured_event(Some("future"), b"{}").is_err());
    }

    #[test]
    fn aoe_version_range_is_pinned() {
        assert!(supported_aoe_version("aoe 1.13.0"));
        assert!(supported_aoe_version("v1.99.2"));
        assert!(!supported_aoe_version("aoe 1.12.9"));
        assert!(!supported_aoe_version("aoe 2.0.0"));
    }

    #[test]
    fn agent_version_fixture_ranges_reject_drift() {
        assert!(supported_agent_version(
            AgentKind::Codex,
            "codex-cli 0.144.5"
        ));
        assert!(!supported_agent_version(
            AgentKind::Codex,
            "codex-cli 0.143.9"
        ));
        assert!(supported_agent_version(
            AgentKind::Claude,
            "2.1.215 (Claude Code)"
        ));
        assert!(!supported_agent_version(AgentKind::Claude, "3.0.0"));
        assert!(supported_agent_version(AgentKind::Antigravity, "1.1.4"));
    }

    #[test]
    fn aoe_authenticated_loopback_lifecycle() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let requests = Arc::new(Mutex::new(Vec::<String>::new()));
        let captured = requests.clone();
        let server = thread::spawn(move || {
            for index in 0..5 {
                let (stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream);
                let mut head = String::new();
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" || line.is_empty() {
                        break;
                    }
                    head.push_str(&line);
                }
                assert!(head.contains("Authorization: Bearer test-token\r\n"));
                let length = head
                    .lines()
                    .find_map(|line| line.strip_prefix("Content-Length: "))
                    .and_then(|value| value.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                captured.lock().unwrap().push(format!(
                    "{}\n{}",
                    head.lines().next().unwrap_or_default(),
                    String::from_utf8_lossy(&body)
                ));
                let response_body = if index == 0 {
                    r#"{"id":"session_1"}"#
                } else if index == 2 {
                    r#"{"status":"running"}"#
                } else if index == 3 {
                    r#"{"output":"bounded"}"#
                } else {
                    r#"{"ok":true}"#
                };
                let mut stream = reader.into_inner();
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    response_body.len(),
                    response_body
                )
                .unwrap();
            }
        });

        let client = AoeClient::loopback(port, "test-token".into()).unwrap();
        assert_eq!(
            client
                .create_session(Path::new("/tmp/worktree"), "codex", "fixture")
                .unwrap()["id"],
            "session_1"
        );
        client.send("session_1", "hello").unwrap();
        assert_eq!(client.status("session_1").unwrap()["status"], "running");
        assert_eq!(client.output("session_1").unwrap()["output"], "bounded");
        client.cancel("session_1").unwrap();
        server.join().unwrap();
        let requests = requests.lock().unwrap();
        assert!(requests[0].contains("POST /api/sessions HTTP/1.1"));
        assert!(requests[0].contains("\"create_new_branch\":false"));
        assert!(requests[1].contains("POST /api/sessions/session_1/send"));
        assert!(requests[2].contains("GET /api/sessions/session_1 HTTP/1.1"));
        assert!(requests[3].contains("lines=200"));
        assert!(requests[4].contains("DELETE /api/sessions/session_1"));
    }

    #[cfg(unix)]
    #[test]
    fn real_adapter_contracts_run_against_quota_free_fake_executable() {
        use std::{os::unix::fs::PermissionsExt, sync::atomic::AtomicBool};
        let dir = tempdir().unwrap();
        let executable = dir.path().join("fake-agent");
        fs::write(
            &executable,
            "#!/bin/sh\nprintf '%s\\n' '{\"type\":\"result\",\"fixture\":true}'\n",
        )
        .unwrap();
        let mut permissions = fs::metadata(&executable).unwrap().permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&executable, permissions).unwrap();

        for agent in [AgentKind::Codex, AgentKind::Claude, AgentKind::Antigravity] {
            let mut invocation = agent
                .build_invocation(executable.clone(), dir.path(), "fixture prompt")
                .unwrap();
            invocation.timeout = Duration::from_secs(2);
            let outcome = run_invocation(&invocation, Arc::new(AtomicBool::new(false))).unwrap();
            assert_eq!(
                outcome.classification,
                crate::process::ExitClassification::Success
            );
            assert!(String::from_utf8_lossy(&outcome.stdout).contains("\"fixture\":true"));
        }
    }

    #[test]
    #[ignore = "real-docker: requires GARNISH_REAL_DOCKER_IMAGE and a healthy local daemon"]
    fn real_docker_backend_create_inspect_run_cleanup() {
        let image = env::var("GARNISH_REAL_DOCKER_IMAGE")
            .expect("set GARNISH_REAL_DOCKER_IMAGE to a locally available digest-pinned image");
        let dir = tempdir().unwrap();
        let name = format!(
            "garnish-conformance-{}",
            ulid::Ulid::new().to_string().to_lowercase()
        );
        let spec = DockerSpec {
            name: name.clone(),
            image,
            worktree: dir.path().to_path_buf(),
            command: vec![
                "/bin/sh".into(),
                "-c".into(),
                "printf container-ok > /workspace/container-output.txt".into(),
            ],
        };
        let backend = DockerBackend::discover().unwrap();
        backend.create(&spec).unwrap();
        let attestation = backend.inspect(&spec).unwrap();
        let output = backend.start_attached(&name).unwrap();
        let cleanup = backend.remove(&name);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(attestation.secure_container, "{:?}", attestation.reasons);
        assert_eq!(
            fs::read_to_string(dir.path().join("container-output.txt")).unwrap(),
            "container-ok"
        );
        cleanup.unwrap();
    }
}
