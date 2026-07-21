# Distributable CLI MVP exit report

## Result

The initial distributable CLI MVP exited successfully on 2026-07-21 at Git revision `495a5c8f2cbee2c4b1d4d9c88bc47137de3ab909` (`MVP phase 3`). Clean-tree native package acceptance passed on Apple Silicon macOS, native x86-64 Linux, and x86-64 WSL2.

This result extends the accepted source-run CLI MVP with an installation contract, task-oriented operator guide, versioned native archives, deterministic archive metadata, SHA-256 manifests, and quota-free acceptance of the extracted executable. It does not claim public-release signing, notarization, package-manager distribution, automatic updates, production readiness, or support for platforms outside the matrix below.

## Platform evidence

| Platform | Archive | Source tree | Result |
| --- | --- | --- | --- |
| Apple Silicon macOS | `harness-garnish-0.1.0-macos-aarch64.tar.gz` | Clean | Passed |
| Native x86-64 Linux | `harness-garnish-0.1.0-linux-x86_64.tar.gz` | Clean | Passed |
| x86-64 WSL2 | `harness-garnish-0.1.0-linux-x86_64.tar.gz` | Clean, Linux-native checkout | Passed |

Each platform completed `scripts/test-release-package` and emitted `Packaged CLI acceptance passed` only after proving:

- two native builds produced the same archive checksum;
- `SHA256SUMS` verified the selected archive;
- extraction produced the expected executable, version, help surface, license, manifest, and packaged guides without symlinks;
- system/global Git configuration, templates, hooks, signing, line-ending conversion, and filesystem monitors could not alter the temporary fixture;
- the extracted executable initialized private state, registered a temporary Git repository, routed and executed a deterministic fake implementation task, and completed detached verification and review;
- source `HEAD` and tracked files remained unchanged, the requested `result.txt` did not enter the registered checkout, and human-readable projection changes remained confined to `.harness-garnish/`;
- the review kept integration unauthorized; and
- Unix state/database modes were `0700` and `0600`.

The tests removed provider credentials and live acknowledgements from the package and task processes. They used no network provider, Codex subscription task, paid API request, container image, or real operator state. Test archives and state were temporary and were removed after acceptance.

## Installation and operation

[`installation.md`](installation.md) is the installation contract for Cargo installation, native archive construction, checksum verification, user-local installation, explicit state location, upgrades, rollback limitations, and uninstall. [`operator-guide.md`](operator-guide.md) covers the complete quota-free fixture, Codex subscription, paid API, read-only dashboard, backup, operational control, and recovery workflows using the installed `garnish` executable.

The persistent packaging command writes the current native archive and checksum manifest beneath `target/release-packages/`. It never tags, commits, publishes, signs, notarizes, or uploads an artifact.

## Residual release scope

- The Linux executable inherits the GNU libc compatibility floor of its native build host.
- macOS artifacts are unsigned and unnotarized.
- Intel macOS, Linux AArch64, and native Windows are outside the accepted matrix.
- Homebrew, Debian/RPM packages, hosted releases, signatures, SBOMs, provenance attestations, service-manager units, and automatic updates remain unimplemented.
- The web interface remains read-only, MCP remains registration-only, and other coding-agent CLIs remain outside the MVP.
- This is not Phase 4 exit or production-readiness evidence.

## Conclusion

The functional and distributable CLI MVP boundaries are complete. Further quota-free packaging repetition or paid-provider smoke tests are not required for this milestone. The next product capability can proceed under a new acceptance boundary without reopening the CLI MVP.
