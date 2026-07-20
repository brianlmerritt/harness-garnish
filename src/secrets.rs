use anyhow::{Context, Result, anyhow, bail};
use std::{
    fmt,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

const MAX_SECRET_BYTES: usize = 16 * 1024;

#[derive(Clone, PartialEq, Eq)]
pub enum SecretReference {
    Environment(String),
    File(PathBuf),
    Keychain { service: String, account: String },
}

impl SecretReference {
    pub fn parse(reference: &str) -> Result<Self> {
        if reference.chars().count() > 200 || reference.chars().any(char::is_whitespace) {
            return Err(invalid_reference());
        }
        if let Some(name) = reference.strip_prefix("env:") {
            let mut bytes = name.bytes();
            if bytes
                .next()
                .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_')
                && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            {
                return Ok(Self::Environment(name.into()));
            }
        } else if let Some(path) = reference.strip_prefix("file:") {
            let path = PathBuf::from(path);
            if path.is_absolute() {
                return Ok(Self::File(path));
            }
        } else if let Some(value) = reference.strip_prefix("keychain:")
            && let Some((service, account)) = value.split_once('/')
            && !service.is_empty()
            && !account.is_empty()
            && [service, account].iter().all(|part| {
                part.bytes().all(|byte| {
                    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'@' | b'-')
                })
            })
        {
            return Ok(Self::Keychain {
                service: service.into(),
                account: account.into(),
            });
        }
        Err(invalid_reference())
    }

    pub fn resolve(&self) -> Result<SecretValue> {
        match self {
            Self::Environment(name) => resolve_environment(name),
            Self::File(path) => resolve_file(path),
            Self::Keychain { service, account } => resolve_keychain(service, account),
        }
    }
}

impl fmt::Debug for SecretReference {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Environment(name) => formatter.debug_tuple("Environment").field(name).finish(),
            Self::File(path) => formatter.debug_tuple("File").field(path).finish(),
            Self::Keychain { service, account } => formatter
                .debug_struct("Keychain")
                .field("service", service)
                .field("account", account)
                .finish(),
        }
    }
}

pub struct SecretValue {
    bytes: Vec<u8>,
}

impl SecretValue {
    fn new(bytes: Vec<u8>) -> Result<Self> {
        let bytes = normalize_secret(bytes)?;
        Ok(Self { bytes })
    }

    pub fn expose<T>(&self, operation: impl FnOnce(&[u8]) -> T) -> T {
        operation(&self.bytes)
    }
}

impl fmt::Debug for SecretValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretValue([REDACTED])")
    }
}

impl Drop for SecretValue {
    fn drop(&mut self) {
        for byte in &mut self.bytes {
            // Volatile clearing makes the best effort explicit without adding a runtime or
            // serialization dependency. SecretValue is never Clone or Serialize.
            unsafe { std::ptr::write_volatile(byte, 0) };
        }
    }
}

fn invalid_reference() -> anyhow::Error {
    anyhow!(
        "API secret reference must be env:NAME, keychain:SERVICE/ACCOUNT, or file:/absolute/path"
    )
}

fn resolve_environment(name: &str) -> Result<SecretValue> {
    let value = std::env::var_os(name)
        .ok_or_else(|| anyhow!("API secret environment reference is unavailable"))?;
    let value = value
        .into_string()
        .map_err(|_| anyhow!("API secret environment value is not valid UTF-8"))?;
    SecretValue::new(value.into_bytes())
}

#[cfg(unix)]
fn resolve_file(path: &Path) -> Result<SecretValue> {
    use std::os::unix::{fs::MetadataExt, fs::OpenOptionsExt};

    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(path)
        .context("opening protected API secret file")?;
    let metadata = file
        .metadata()
        .context("inspecting protected API secret file")?;
    if !metadata.file_type().is_file() {
        bail!("API secret file must be a regular file");
    }
    if metadata.uid() != unsafe { libc::geteuid() } {
        bail!("API secret file must be owned by the Garnish user");
    }
    if metadata.mode() & 0o077 != 0 {
        bail!("API secret file must not grant group or other permissions");
    }
    read_bounded_secret(&mut file)
}

#[cfg(not(unix))]
fn resolve_file(_path: &Path) -> Result<SecretValue> {
    bail!("protected API secret files are not supported on this operating system")
}

fn read_bounded_secret(file: &mut File) -> Result<SecretValue> {
    let mut bytes = Vec::new();
    file.take((MAX_SECRET_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .context("reading protected API secret file")?;
    SecretValue::new(bytes)
}

#[cfg(target_os = "macos")]
fn resolve_keychain(service: &str, account: &str) -> Result<SecretValue> {
    use crate::process::{ExitClassification, ProcessSpec, supervise};
    use std::{
        collections::BTreeMap,
        sync::{Arc, atomic::AtomicBool},
        time::Duration,
    };

    let executable = Path::new("/usr/bin/security");
    let argv = vec![
        std::ffi::OsString::from("find-generic-password"),
        std::ffi::OsString::from("-s"),
        std::ffi::OsString::from(service),
        std::ffi::OsString::from("-a"),
        std::ffi::OsString::from(account),
        std::ffi::OsString::from("-w"),
    ];
    let environment = BTreeMap::new();
    let outcome = supervise(
        ProcessSpec {
            executable,
            argv: &argv,
            cwd: Path::new("/"),
            environment: &environment,
            stdin: &[],
            timeout: Duration::from_secs(5),
            termination_grace: Duration::from_secs(1),
            output_limit: MAX_SECRET_BYTES + 1,
        },
        Arc::new(AtomicBool::new(false)),
    )
    .context("resolving macOS Keychain API secret reference")?;
    if outcome.classification != ExitClassification::Success
        || outcome.stdout_truncated
        || outcome.stderr_truncated
    {
        bail!("macOS Keychain API secret reference is unavailable");
    }
    SecretValue::new(outcome.stdout)
}

#[cfg(not(target_os = "macos"))]
fn resolve_keychain(_service: &str, _account: &str) -> Result<SecretValue> {
    bail!("keychain API secret references require macOS")
}

fn normalize_secret(mut bytes: Vec<u8>) -> Result<Vec<u8>> {
    if bytes.ends_with(b"\r\n") {
        bytes.truncate(bytes.len() - 2);
    } else if bytes.ends_with(b"\n") {
        bytes.pop();
    }
    if bytes.is_empty() || bytes.len() > MAX_SECRET_BYTES {
        bail!("API secret value must contain 1..={MAX_SECRET_BYTES} bytes");
    }
    if !bytes.iter().all(u8::is_ascii_graphic) {
        bail!("API secret value contains unsupported whitespace or control bytes");
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    const CANARY: &str = "secret-canary-never-log-7f18c99a";

    #[test]
    fn references_are_exact_and_never_accept_a_value() {
        assert!(matches!(
            SecretReference::parse("env:OPENAI_API_KEY").unwrap(),
            SecretReference::Environment(_)
        ));
        assert!(matches!(
            SecretReference::parse("keychain:harness-garnish/openai-primary").unwrap(),
            SecretReference::Keychain { .. }
        ));
        assert!(SecretReference::parse("file:/private/secret/api-key").is_ok());
        for invalid in [
            CANARY,
            "env:sk-ant-secret",
            "env:",
            "file:relative",
            "keychain:missing-account",
            "keychain:service/account/extra",
        ] {
            let error = SecretReference::parse(invalid).unwrap_err().to_string();
            assert!(!error.contains(CANARY));
        }
    }

    #[cfg(unix)]
    #[test]
    fn protected_file_resolution_rejects_permissions_and_symlinks_and_redacts_debug() {
        use std::os::unix::fs::{PermissionsExt, symlink};

        let directory = tempdir().unwrap();
        let secret_path = directory.path().join("api-key");
        std::fs::write(&secret_path, format!("{CANARY}\n")).unwrap();
        std::fs::set_permissions(&secret_path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let reference = SecretReference::File(secret_path.clone());
        let secret = reference.resolve().unwrap();
        assert!(secret.expose(|bytes| bytes == CANARY.as_bytes()));
        assert_eq!(format!("{secret:?}"), "SecretValue([REDACTED])");
        assert!(!format!("{secret:?}").contains(CANARY));
        drop(secret);

        std::fs::set_permissions(&secret_path, std::fs::Permissions::from_mode(0o640)).unwrap();
        let error = reference.resolve().unwrap_err().to_string();
        assert!(!error.contains(CANARY));

        std::fs::set_permissions(&secret_path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let link_path = directory.path().join("api-key-link");
        symlink(&secret_path, &link_path).unwrap();
        let error = SecretReference::File(link_path)
            .resolve()
            .unwrap_err()
            .to_string();
        assert!(!error.contains(CANARY));
    }

    #[test]
    fn malformed_values_are_rejected_without_echoing_the_canary() {
        for value in [Vec::new(), format!("{CANARY}\nsecond-line").into_bytes()] {
            let error = SecretValue::new(value).unwrap_err().to_string();
            assert!(!error.contains(CANARY));
        }
    }
}
