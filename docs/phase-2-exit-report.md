# Phase 2 exit report

## Result

Phase 2 exited successfully on 2026-07-20. The schema-7 durable scheduler and recovery implementation passed its normal quota-free suite on macOS, a Linux VPS, and Ubuntu 24.04 under WSL2. This is a Phase 2 acceptance result, not a production-readiness or real-provider-spend claim.

## Final platform evidence

### macOS development host

- Formatting and strict Clippy completed without warnings.
- Rust library suite: 68 passed, 0 failed, 2 explicitly ignored opt-in real-container tests.
- CLI integration suite: 3 passed, 0 failed.
- MVP vertical slice: 1 passed, 0 failed.
- Authenticated AoE loopback, process supervision, schema migration, scheduling, recovery, policy, quota, and Git-isolation tests passed.

### Linux VPS

- Kernel: Linux `6.8.0-107-generic` on `x86_64`.
- Rust and Cargo: `1.97.1`.
- Rust library suite: 68 passed, 0 failed, 2 explicitly ignored opt-in real-container tests.
- CLI integration suite: 3 passed, 0 failed.
- MVP vertical slice: 1 passed, 0 failed.
- Bounded daemon shutdown completed through `max_ticks`; signal-driven shutdown completed through `TERM` with `shutdown_reason: signal`.
- State directory and database modes were `0700` and `0600`.
- Rootless Podman and Docker runtime probes were healthy. Their opt-in conformance tests were skipped in this final scheduler rerun because the backend implementation had not changed; both backends had already passed digest-pinned create/inspect/run/cleanup conformance on this host.

### WSL2

- Distribution: Ubuntu 24.04; kernel `6.6.87.2-microsoft-standard-WSL2` on `x86_64`.
- Rust and Cargo: `1.97.1`.
- Checkout: Linux-native filesystem at `/home/blm/dev/docker/harness-garnish`.
- Rust library suite: 68 passed, 0 failed, 2 explicitly ignored opt-in real-container tests.
- CLI integration suite: 3 passed, 0 failed.
- MVP vertical slice: 1 passed, 0 failed.
- Windows-mounted project roots remained denied by default; signals, descendant cleanup, restart recovery, runtime selection, and private file modes passed.
- Rootless Podman was selected and healthy. It warned that no systemd user session was available and used its documented `cgroupfs` fallback. Docker was not installed and was not required because a supported healthy runtime was present.
- The schema-7 online backup passed integrity checking, had mode `0600`, and emitted SHA-256 `d61c41ae6d0f884f388368b9ab8e0ff2c5b2848860287bcef31fe6f0b2bdcab5`.

No provider subscription quota or API budget was consumed by these bundles.

## Acceptance mapping

| Acceptance | Final evidence |
| --- | --- |
| P2-01 | Ordered schema-1-to-7 migration creates and verifies a backup and preserves canonical rows. |
| P2-02–P2-03 | Work/off/both combinations, IANA timezone, DST, and dated exceptions pass deterministic tests. |
| P2-04 | Dependency, project pause, calendar, quota, policy, retry, deadline, capability, capacity, and project-lock exclusions persist stable machine reason codes. |
| P2-05–P2-06 | Restart-stable priority/deadline ordering and atomic global, adapter, account, and project ceilings pass, including racing schedulers. |
| P2-07 | Crash/reopen boundaries before claim, after unconsumed claim, after consumed action/run, and after cleanup recover at most once without duplicating an action or run. |
| P2-08–P2-09 | TERM/KILL process-tree evidence, retry budgets, deterministic backoff, and circuit breakers pass on all three environments. |
| P2-10–P2-11 | Day-boundary and live quota/policy/calendar checkpoint outcomes pass with durable supervision state. |
| P2-12 | The normal fake-adapter suite passes on macOS and Linux without provider calls. |
| P2-13 | The refreshed WSL2 bundle passes path policy, signals, permissions, recovery, backup, and runtime selection. |
| P2-14 | Repository policy continues to deny scheduler-created branches and other automated Git integration for this user-managed repository. |

## Residual scope

Phase 3 owns multi-agent capability routing, real quota/history adapters, quota reservations, manual agent pinning, and independent verification. Real Codex, Claude Code, Antigravity, CodexBar, and provider smoke tests remain explicit opt-ins because they can touch authentication state or consume subscription quota. API/local-model providers, Skills/MCP/ACP lifecycle, remote approvals, native notification delivery, packaging, automatic updates, encrypted portable export, and Apple Container conformance remain later phases.
