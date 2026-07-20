# Phase 2 plan — durable scheduler and recovery

## Objective

Phase 2 turns the synchronous Phase 1 control plane into a durable local scheduler. It must select eligible work deterministically, survive restart, prevent duplicate external actions, checkpoint and pause safely, and explain every scheduling decision without consuming provider quota in its normal test suite.

## Day-aware scheduling

Day awareness is a first-class scheduler gate, not an agent preference.

- A calendar profile has an IANA timezone and a seven-character weekly pattern in Monday-to-Sunday order. Each character is `W` (user workday) or `O` (user off day). The default is `WWWWWOO`.
- A task has a day affinity: `W` (run on workdays), `O` (run on off days), or `B` (both). The default is `B` for backward compatibility.
- A dated exception can reclassify one local date as `W` or `O`, with a reason. Exceptions support holidays, leave, and unusual workdays without rewriting the weekly pattern.
- The scheduler evaluates the date in the profile timezone. UTC storage remains canonical; DST and local-midnight calculations use the retained IANA timezone.
- An ineligible queued task remains ready and receives an exact next eligible wake time plus a recorded explanation.
- If a running task crosses into an ineligible day, Garnish checkpoints and pauses it at the next safe checkpoint. It does not kill a process at local midnight merely because the date changed. Emergency policy remains immediate.
- Quota reset wake times, retry backoff, deadlines, dependencies, resource locks, and calendar eligibility are combined by taking the earliest safe time at which every hard gate may pass.

Calendar examples:

| Calendar | Task affinity | Monday | Saturday |
| --- | --- | --- | --- |
| `WWWWWOO` | `W` | eligible | ineligible |
| `WWWWWOO` | `O` | ineligible | eligible |
| `WWWWWOO` | `B` | eligible | eligible |

## Implementation stages

1. Schema versions 2–7: calendar profiles, project-to-calendar selection, dated exceptions, task affinity, scheduler instances, leader fencing, durable wake records, task claims, resource locks, claim-to-run bindings, checkpoints, retry state, adapter circuits, operational controls, local notifications, project scheduler pauses, deadlines, capability requirements, typed route reasons, and route-specific capacity slots. Prove version-1 backup and migration integrity. Ordered development migrations remain separate so an early Phase 2 database can advance safely.
2. Pure scheduler kernel: injected clock, ready-set calculation, deterministic ordering, eligibility reasons, next wake, concurrency ceilings, and atomic lease claims.
3. Local daemon: singleton leader lease, heartbeats, graceful shutdown, stale-leader recovery, orphan cleanup, and idempotency claims around external actions.
4. Runtime supervision: checkpoint timers, cancellation, retry budgets, stable failure categories, exponential backoff with jitter derived from persisted state, and adapter circuit breakers.
5. Live changes: re-evaluate queued work immediately after quota, policy, calendar, or user override changes; checkpoint/pause active runs when required.
6. Notifications and operations: local review/block/failure notices, status/stop/emergency-stop commands, backup/export, and bounded diagnostics.

## Phase 2 machine acceptance

| ID | Assertion |
| --- | --- |
| P2-01 | Opening a schema-1 fixture creates and integrity-checks a backup, applies every ordered Phase 2 migration exactly once, and preserves every Phase 1 row/event. |
| P2-02 | All 21 combinations of weekday class (`W`/`O`), task affinity (`W`/`O`/`B`), and representative weekdays yield deterministic eligibility. |
| P2-03 | IANA timezone and DST fixtures compute the correct local date and next eligible instant; a dated exception overrides only its target date. |
| P2-04 | Dependency, project pause, calendar, quota, policy, retry, deadline, lock, and capability exclusions are all recorded with stable reason codes. |
| P2-05 | Priority/deadline ordering is deterministic across restart; concurrency and per-agent/account/resource ceilings are never exceeded. |
| P2-06 | Two schedulers racing for one task produce one lease and one external-action claim. |
| P2-07 | Kill/reopen tests around each material transition never duplicate a claimed action; stale leases recover once. |
| P2-08 | TERM then KILL cancellation cleans descendants within the configured bound and persists classification/evidence. |
| P2-09 | Retry budgets, deterministic backoff, and circuit breakers stop retry storms and expose the next wake time. |
| P2-10 | A day-boundary change checkpoints and pauses an ineligible active task; `B` tasks continue. |
| P2-11 | Mid-run quota/policy/calendar changes produce continue, shorten-checkpoint, pause, or emergency-stop outcomes as configured. |
| P2-12 | Normal CI uses fake adapters only, emits no network/provider call, and passes on macOS and Linux. |
| P2-13 | WSL2 tests cover Linux paths, default denial for Windows-mounted worktrees, signals, permissions, restart, and Docker/Podman discovery. |
| P2-14 | This repository's user-managed Git policy still prevents scheduler-created branches, commits, pulls, pushes, PRs, and merges. |

## Platform milestones

- macOS: implement schema, pure scheduler, daemon core, and fake-runtime tests first.
- Linux midpoint: before daemon lifecycle is considered stable, run scheduler leadership, signals, process cleanup, filesystem permissions, and rootless runtime probes on Ubuntu or Debian.
- WSL2 exit: before Phase 2 exit, run the WSL2 bundle for mounted-path policy, process cleanup, restart recovery, and runtime selection.
- Native Windows is not a Phase 2 target; WSL2 is the Windows execution environment.

## Current implementation checkpoint

- Schema migrations, calendar scheduling, deterministic ready-set routing, leader fencing, atomic task claims, global concurrency, and exclusive project locks are implemented.
- The local daemon now renews leadership and active claims, performs scheduler/run-lease recovery, handles `TERM`/`INT`, and releases unstarted claims on graceful shutdown.
- State directories and SQLite database files are restricted to the owning user on Unix.
- `scripts/test-linux-midpoint` passed the scheduler, runtime-supervision, signal, cleanup, permission, and rootless-Podman probe bundle on a Linux VPS; see `phase-2-linux-midpoint.md`.
- The opt-in quota-free fake path now binds a route and scheduler claim atomically to one run and one single-use external-action key. Completed and orphaned runs release project locks, and clean prepared worktrees can be adopted after a pre-consumption restart.
- Schema 5 and the runtime-supervision core are implemented locally: lease-fenced checkpoint sequences and heartbeats, durable cancellation intent, TERM-to-KILL process-tree evidence, stable failure categories, bounded retry budgets with deterministic exponential jitter, persisted retry wake gates, and per-adapter/account circuit breakers with a single half-open probe.
- Active-run checkpoint evaluation now covers day eligibility, live quota headroom, policy revocation, continue, shortened interval, graceful pause, and cancellation. A pause/cancel decision retains ownership until process termination is acknowledged, then releases the run lease and project lock.
- Schema 6 adds durable global pause/emergency-stop state and a bounded local notification outbox. Operational status, diagnostics, explicit resume, notification acknowledgement, and private integrity-checked state backup are exposed through the CLI. Emergency stop atomically blocks new claims, releases unstarted claims, and requests—not falsely completes—active-run cancellation.
- Schema 7 adds durable per-project scheduler pause state, task deadlines and capability requirements, stable route reason codes, and adapter/account capacity slots. The scheduler records dependency, project pause, calendar, quota, policy, retry, expired deadline, capability, capacity, and project-lock exclusions without parsing human-readable rationale. This completes P2-04 locally.
- Priority then earliest-deadline ordering is covered across database restart. Global, per-adapter, per-account, and exclusive project ceilings are transactionally enforced and released during recovery. This completes P2-05 locally.
- A durable crash/reopen matrix now covers before claim, unconsumed claim, consumed action/run, and completed cleanup boundaries. Each boundary recovers at most once and never duplicates the external-action key or run. This completes P2-07 locally.
- WSL2 defaults to denying project roots under `/mnt/<drive>`. The quota-free exit bundle passed on Ubuntu 24.04 under WSL2 with Linux-native paths, lifecycle/restart checks, private state and backup permissions, and rootless Podman selection; see `phase-2-wsl2-exit.md`. This supplies P2-13 evidence.
- Hardened rootless-Podman and Docker backends have fail-closed runtime/effective-state attestation. Both explicit opt-in create/inspect/run/cleanup tests passed on Ubuntu 24.04.4 using the same digest-pinned Alpine image; see `phase-2-linux-container-conformance.md`.
- The Phase 2 implementation and machine-acceptance coverage are locally complete. Phase 2 exit still requires the refreshed Linux midpoint and WSL2 exit bundles to pass after schema 7 is pulled. Real-agent claim execution, native desktop notification delivery, encrypted portable export, and Apple Container conformance remain later integration work and are not silently treated as complete.

## Explicit non-goals

Phase 2 does not enable real API spending, remote workers, Tailscale/SSH approvals, automatic updates, or autonomous Git integration. Real provider and AoE smoke tests remain individually opt-in.
