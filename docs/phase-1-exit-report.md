# Phase 1 exit report

## Status

The Phase 1 local vertical slice is implemented and passes its macOS arm64 machine tests. This is not a production-readiness claim. Linux CI is defined but cannot produce remote evidence until the user chooses to commit/push the workflow; WSL2 execution and real provider/AoE smoke tests remain explicitly unverified.

No provider subscription quota or API budget was consumed. No source-repository commit, branch switch, push, pull request, merge, remote edit, or submodule update was performed by the implementation session. The user remains responsible for this repository's Git operations.

## Implemented scope

- Rust `garnish` CLI with JSON success/error output and documented exit behavior.
- SQLite schema version 1 with WAL, foreign keys, busy timeout, transactional events, dependency graph, quota observations/overrides, routes, runs, leases, approvals, and pre-migration backup framework.
- Stable project registry, explicit parent/child project links, deterministic global backlog, dependency waiting/promotion, and compare-and-swap state transitions.
- Project projections for `PROJECT.md`, `MEMORY.md`, `TASKS.md`, `HANDOFF.md`, generated agent context, run links, SHA-256 sidecars, and stale-edit conflict rejection.
- Per-surface quota routing with 20% default reserves, forecast headroom, unknown fail-closed behavior, live scoped overrides, reset wake time, candidate rationale, and policy hash.
- Deterministic fake agent/execution/sandbox adapters; bounded machine evidence; separate task and detached verification worktrees; independent command exit evaluation.
- Codex, Claude Code, and Antigravity version probes, compatibility fixtures, argv-only invocation builders, structured/text parsers, bounded process supervision, and quota-free fake-executable conformance.
- Authenticated loopback AoE client for session create, send, status, bounded output, and cancellation; supported range `>=1.13.0,<2.0.0`; fake HTTP lifecycle fixture.
- Docker create/inspect/start/remove backend using digest-pinned images, no network, read-only root, dropped capabilities, `no-new-privileges`, non-root user, CPU/memory/PID limits, a sole project bind mount, and effective-state attestation.
- Timeout and cancellation escalation to the complete Unix process group, descendant cleanup evidence, and bounded stdout/stderr retention.
- Single-use, expiring, action-digest-bound approvals for representative Class 2/3 effects; OpenAI and Anthropic API default deny; user-managed Git policy denial before branch/worktree creation.
- Local notification interface with deterministic fake adapter. Tailscale and SSH delivery remain deferred.

## Machine evidence

Local platform and toolchain on 2026-07-19:

| Component | Tested value |
| --- | --- |
| Host | Darwin 25.5.0 arm64 |
| Rust / Cargo | 1.97.1 / 1.97.1 |
| Git | 2.52.0 |
| Codex CLI | 0.144.5 |
| Claude Code | 2.1.215 |
| Antigravity CLI | 1.1.4 |
| AoE | 1.13.0 |
| Docker client/server | 29.6.1 / 29.6.1 |
| SQLite schema | 1 (bundled SQLite through `rusqlite`) |

Commands and results:

```console
cargo fmt --all -- --check
# pass

cargo clippy --workspace --all-targets -- -D warnings
# pass

cargo test --workspace
# pass: unit, CLI JSON integration, and MVP vertical-slice integration

GARNISH_REAL_DOCKER_IMAGE='postgres@sha256:be01cf82fc7dbba824acf0a82e150b4b360f3ff93c6631d7844af431e841a95c' \
  cargo test adapters::tests::real_docker_backend_create_inspect_run_cleanup -- --ignored
# pass: cached image; container created, inspected, executed, and removed
```

The MVP test registers two fixture repositories, persists a parent/child topology, creates dependent tasks, restarts Garnish, declines low five-hour quota with a wake time, applies a live override without changing the observation, routes through two named fake adapters, creates isolated task and verifier worktrees, emits review artifacts, and proves source `HEAD`, current branch, and meaningful status remain unchanged.

## Acceptance coverage and limitations

The automated suite directly covers MVP-01 through MVP-07 and MVP-13 through MVP-15. MVP-08 is split between digest-ordered event evidence and process-tree timeout/cancellation tests. MVP-09 proves SQLite reopen plus idempotent orphan-lease recovery, but the broader crash-injection matrix is deferred to Phase 2 as allowed by the acceptance plan. MVP-10 proves explicit handoff data and adapter-independent invocation contracts; proprietary conversation state is neither stored nor transferred. MVP-11 uses a detached clean verification worktree and independent exit status. MVP-12's local review bundle contains patch, manifests, logs, route/quota snapshots, verification, handoff, digests, and integration instructions; richer support-bundle export remains future work.

Unverified or deliberately deferred:

- Real Codex, Claude Code, and Antigravity smoke runs: opt-in because they may consume subscription quota.
- Real AoE server lifecycle: opt-in; the authenticated fake service covers the normal suite. Stock AoE sandbox defaults are not considered secure and are not used as attestation.
- Linux workflow execution awaits a user-managed commit/push. WSL2 remains a documented target without local execution evidence.
- Podman and Apple Container probes/conformance, real quota collectors, API calls and hard-spend enforcement, remote notifications, daemon scheduling, broad retry/circuit-breaker behavior, richer UI, and update activation remain in later phases.
- Docker conformance used an already-cached Postgres image only as a portable shell fixture; it does not endorse that image for agent workloads. A production image, SBOM/signature policy, disk quota enforcement, and platform-specific secret projection still need definition.
- The current CLI runs the fake vertical slice synchronously. The supervised real-agent runner is implemented and fixture-tested but not connected to unattended production execution until secure-container credential projection and opt-in smoke evidence exist.

## Security attestation result

The real Docker fixture passed effective inspection for network `none`, read-only root, all capabilities dropped, `no-new-privileges`, user `1000:1000`, two CPUs, 2 GiB memory, 256 PIDs, no runtime socket, no host-home mount, and exactly one writable host bind mount (the fixture worktree). The fake backend records the same policy fields deterministically for the end-to-end slice. Neither result proves protection from unknown runtime vulnerabilities.

## Reproduction and integration request

Run the three default commands above. The real Docker test is optional and requires a locally cached digest-pinned image plus an accessible daemon. Review all working-tree changes and generated test evidence, then use the user's normal manual branch/commit process. Do not infer authorization to commit, push, open a PR, or merge from this report.
