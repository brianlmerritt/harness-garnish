# Phase 2 WSL2 exit evidence

## Result

The dedicated quota-free WSL2 bundle passed on a user-provided Ubuntu 24.04 environment on 2026-07-20. The checkout was on the WSL2 Linux filesystem at `/home/blm/dev/docker/harness-garnish`, not a Windows drive mount.

## Environment

- Kernel: Linux `6.6.87.2-microsoft-standard-WSL2` on `x86_64`.
- Distribution: Ubuntu 24.04.
- Rust and Cargo: `1.97.1`.
- Runtime selected: rootless Podman.
- Docker: not installed.
- Provider subscription/API usage: none.

## Captured evidence

- Rust library suite: 57 passed, 0 failed, 1 explicitly ignored opt-in real-Docker test.
- CLI suite: 3 passed, 0 failed.
- MVP vertical slice: 1 passed, 0 failed.
- AoE authenticated loopback, daemon bounded shutdown, `TERM` handling, process-tree cleanup, lease/restart recovery, migrations, retry/circuit behavior, and runtime-supervision tests passed.
- The WSL2-specific mounted-path policy test denied `/mnt/<drive>` project roots by default and allowed Linux-native roots.
- Rootless `podman info` succeeded and the runtime-selection check selected Podman.
- Operational schema-6 status reported no active work and no emergency stop.
- SQLite online backup passed `integrity_check`, was 294,912 bytes, and emitted SHA-256 `3e139a97aff50d9d02808df094cef928954bda3d25c47ce4d9b80252203d6465`.
- State directory, database, and backup modes were `0700`, `0600`, and `0600`.
- The script ended with `WSL2 exit bundle passed`.

## Runtime caveat

Podman warned that its configured systemd cgroup manager had no user session and fell back to `cgroupfs`. That fallback did not fail the capability probe or the quota-free suite. It remains relevant to resource-control attestation: this result proves WSL2 runtime discovery and a healthy rootless Podman control plane, not the full Podman container sandbox matrix or enforcement of every requested cgroup limit.

## Acceptance mapping

This run supplies the platform evidence required by P2-13: Linux-native WSL paths, default Windows-mounted-path denial, signals and descendant cleanup, permissions, restart recovery, and runtime discovery/selection. The Podman conformance test is now implemented but was not part of this captured run; rerun the bundle with `GARNISH_REAL_PODMAN_IMAGE` set to a local digest-pinned image to add that evidence. Docker conformance remains separate and explicitly opt-in.
