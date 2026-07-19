use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    path::Path,
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExitClassification {
    Success,
    Failed,
    TimedOut,
    Cancelled,
    Signalled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessOutcome {
    pub classification: ExitClassification,
    pub exit_code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
    pub elapsed_ms: u128,
}

pub struct ProcessSpec<'a> {
    pub executable: &'a Path,
    pub argv: &'a [std::ffi::OsString],
    pub cwd: &'a Path,
    pub environment: &'a BTreeMap<String, String>,
    pub stdin: &'a [u8],
    pub timeout: Duration,
    pub termination_grace: Duration,
    pub output_limit: usize,
}

pub fn supervise(spec: ProcessSpec<'_>, cancelled: Arc<AtomicBool>) -> Result<ProcessOutcome> {
    if spec.output_limit == 0 {
        bail!("process output limit must be greater than zero");
    }
    let started = Instant::now();
    let mut command = Command::new(spec.executable);
    command
        .args(spec.argv)
        .current_dir(spec.cwd)
        .env_clear()
        .envs(spec.environment)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    configure_process_group(&mut command);
    let mut child = command
        .spawn()
        .with_context(|| format!("starting {}", spec.executable.display()))?;
    let pid = child.id();
    let stdout = child.stdout.take().context("capturing process stdout")?;
    let stderr = child.stderr.take().context("capturing process stderr")?;
    let stdout_reader = bounded_reader(stdout, spec.output_limit);
    let stderr_reader = bounded_reader(stderr, spec.output_limit);
    if let Some(mut input) = child.stdin.take() {
        input.write_all(spec.stdin)?;
    }

    let classification = loop {
        if let Some(status) = child.try_wait()? {
            break if status.success() {
                ExitClassification::Success
            } else if status.code().is_none() {
                ExitClassification::Signalled
            } else {
                ExitClassification::Failed
            };
        }
        if cancelled.load(Ordering::SeqCst) {
            terminate_tree(pid, &mut child, spec.termination_grace)?;
            break ExitClassification::Cancelled;
        }
        if started.elapsed() >= spec.timeout {
            terminate_tree(pid, &mut child, spec.termination_grace)?;
            break ExitClassification::TimedOut;
        }
        thread::sleep(Duration::from_millis(10));
    };
    let status = child.wait()?;
    let (stdout, stdout_truncated) = stdout_reader
        .join()
        .map_err(|_| anyhow::anyhow!("stdout reader panicked"))??;
    let (stderr, stderr_truncated) = stderr_reader
        .join()
        .map_err(|_| anyhow::anyhow!("stderr reader panicked"))??;
    Ok(ProcessOutcome {
        classification,
        exit_code: status.code(),
        stdout,
        stderr,
        stdout_truncated,
        stderr_truncated,
        elapsed_ms: started.elapsed().as_millis(),
    })
}

fn bounded_reader(
    mut reader: impl Read + Send + 'static,
    limit: usize,
) -> thread::JoinHandle<std::io::Result<(Vec<u8>, bool)>> {
    thread::spawn(move || {
        let mut retained = Vec::with_capacity(limit.min(64 * 1024));
        let mut buffer = [0_u8; 8192];
        let mut truncated = false;
        loop {
            let count = reader.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            let available = limit.saturating_sub(retained.len());
            retained.extend_from_slice(&buffer[..count.min(available)]);
            truncated |= count > available;
        }
        Ok((retained, truncated))
    })
}

#[cfg(unix)]
fn configure_process_group(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

#[cfg(not(unix))]
fn configure_process_group(_command: &mut Command) {}

#[cfg(unix)]
fn terminate_tree(pid: u32, child: &mut std::process::Child, grace: Duration) -> Result<()> {
    // The child was placed in its own process group, so negative PID signals include descendants.
    unsafe {
        libc::kill(-(pid as i32), libc::SIGTERM);
    }
    let deadline = Instant::now() + grace;
    while Instant::now() < deadline {
        if child.try_wait()?.is_some() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(10));
    }
    unsafe {
        libc::kill(-(pid as i32), libc::SIGKILL);
    }
    Ok(())
}

#[cfg(not(unix))]
fn terminate_tree(_pid: u32, child: &mut std::process::Child, grace: Duration) -> Result<()> {
    child.kill()?;
    let deadline = Instant::now() + grace;
    while Instant::now() < deadline && child.try_wait()?.is_none() {
        thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn spec<'a>(
        cwd: &'a Path,
        argv: &'a [std::ffi::OsString],
        environment: &'a BTreeMap<String, String>,
    ) -> ProcessSpec<'a> {
        ProcessSpec {
            executable: Path::new("/bin/sh"),
            argv,
            cwd,
            environment,
            stdin: b"",
            timeout: Duration::from_millis(120),
            termination_grace: Duration::from_millis(50),
            output_limit: 128,
        }
    }

    #[test]
    fn timeout_kills_descendant_process_group() {
        let dir = tempdir().unwrap();
        let marker = dir.path().join("orphan-marker");
        let script = format!("(sleep 0.4; touch '{}') & sleep 5", marker.display());
        let argv = vec!["-c".into(), script.into()];
        let environment = BTreeMap::new();
        let outcome = supervise(
            spec(dir.path(), &argv, &environment),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        assert_eq!(outcome.classification, ExitClassification::TimedOut);
        thread::sleep(Duration::from_millis(500));
        assert!(
            !marker.exists(),
            "descendant survived process-group cancellation"
        );
    }

    #[test]
    fn cancellation_and_output_are_bounded() {
        let dir = tempdir().unwrap();
        let argv = vec!["-c".into(), "while :; do printf x; done".into()];
        let environment = BTreeMap::new();
        let cancelled = Arc::new(AtomicBool::new(false));
        let trigger = cancelled.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(50));
            trigger.store(true, Ordering::SeqCst);
        });
        let outcome = supervise(spec(dir.path(), &argv, &environment), cancelled).unwrap();
        assert_eq!(outcome.classification, ExitClassification::Cancelled);
        assert_eq!(outcome.stdout.len(), 128);
        assert!(outcome.stdout_truncated);
    }
}
