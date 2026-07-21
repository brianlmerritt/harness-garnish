# MVP and Phase 1 acceptance tests

## Purpose

Phase 1 must deliver a tested vertical slice, not a command scaffold. CI uses fake agents and fake quota sources and consumes no provider subscription or API budget. Real Codex/Claude/Antigravity and runtime smoke tests are opt-in and clearly disclose expected external effects.

The intended automation entry point is `cargo test --workspace`; exact package/test names may be refined when the Rust workspace is created. A separate `scripts/demo-mvp` or equivalent deterministic command will run the user-facing demonstration.

## Vertical-slice scenario

Given two temporary Git fixture projects and a clean temporary Garnish data directory:

1. initialise Garnish state;
2. register both projects and identify one as an overarching project referencing the other;
3. add two dependent tasks with explicit acceptance criteria, verification argv, risk, estimate, and five-minute-or-shorter checkpoint policy;
4. return a low fake five-hour quota surface and prove scheduling declines the dependent implementation task with a recorded next wake time;
5. append a scoped user quota override and return healthy weekly/monthly surfaces;
6. route the ready task to a deterministic fake adapter with full recorded rationale;
7. create an isolated task worktree and fake/real constrained backend with attestation;
8. run the fake agent, stream versioned events, modify one declared file, checkpoint, and exit;
9. restart the control-plane process after a persisted checkpoint and prove recovery from SQLite without duplicate execution;
10. construct a handoff and resume through a second fake adapter without transferring hidden conversation state;
11. verify in a newly created clean sandbox using the declared command;
12. present patch/branch reference, run manifest, logs, verification, approvals, quota observations/overrides/reservations, and remaining risks;
13. do not commit, push, merge, alter a remote, or modify the source checkout.

The scenario passes only when all assertions below pass.

## Required acceptance matrix

| ID | Requirement | Machine assertion |
| --- | --- | --- |
| MVP-01 | Register two projects and global backlog | JSON list contains two stable project IDs; task priority ordering is deterministic across restart |
| MVP-02 | Dependent task contracts | Invalid/missing criteria fail validation; dependency cycle insertion is rejected atomically; dependent task is not ready early |
| MVP-03 | Quota decline/reschedule | A five-hour surface below reserve+forecast yields `quota_headroom` rejection and next reset wake time; weekly/monthly surfaces remain separately visible |
| MVP-04 | Live quota override | Scoped override changes effective percentage without mutating snapshot; event records actor/reason/expiry; queued task is re-evaluated |
| MVP-05 | Recorded routing | Decision includes all candidates, filter reasons, score inputs, exact snapshot/override IDs, policy hash, and selected adapter |
| MVP-06 | Isolated worktree | Source checkout bytes/status unchanged; task worktree is owned by task/run and based on recorded commit |
| MVP-07 | Secure sandbox | Attestation proves only allowed RW mount, no home/SSH/socket mount, scoped env, network policy, resource limits, runtime/image evidence; otherwise run is denied |
| MVP-08 | Agent evidence and cancellation | JSONL events are ordered and bounded; cancellation terminates descendants within configured grace; status and exit classification persist |
| MVP-09 | Restart recovery | Kill after every material transition; restart yields one legal task state, expires orphan leases, and never duplicates a completed external action |
| MVP-10 | Handoff/adaptor switch | Handoff contains repository/evidence fields and no chain-of-thought field; second adapter resumes from worktree plus handoff |
| MVP-11 | Independent verification | Verifier uses a different clean sandbox and produced commit/patch; fake agent's false “tests pass” cannot override a failing exit code |
| MVP-12 | Review bundle | Bundle contains diff, commits/patch refs, command/tool versions, logs/digests, approvals, quota/spend, unresolved risks, and exact integration request |
| MVP-13 | No external Git effect | No push, PR, merge, remote edit, source-checkout commit, branch switch, or submodule update occurs |
| MVP-14 | State/projections | SQLite is canonical; generated project files have schema/hash; stale human edit reports conflict with no partial import |
| MVP-15 | API default deny | OpenAI/Anthropic API invocation is rejected unless project provider and budget are enabled; no network call occurs in the denial test |

## Phase 1 implementation scope

### Required

- Rust CLI named `garnish` with stable JSON mode and documented exit codes.
- SQLite migration/backup framework and project/task/run/event tables needed by the slice.
- Global project registry and backlog.
- Validated task schema, dependency graph, and idempotent state machine.
- Project projections sufficient for `PROJECT.md`, `MEMORY.md`, `TASKS.md`, `HANDOFF.md`, and run evidence.
- Fake deterministic agent, quota provider, execution plane, and sandbox backend.
- Real execution adapters for all three locally selected agents: structured Codex, structured Claude Code, and supervised text/PTY Antigravity. Real-provider smoke tests remain opt-in, but each adapter must pass version/help/output fixtures and fake-executable conformance in normal CI.
- AoE adapter for probe plus the smallest session lifecycle supported by the selected pinned release.
- Docker backend plus full fake backend. Podman/Apple Container interfaces and probes may exist, but support is not claimed until their conformance suites pass.
- Worktree-per-task handling with dirty-source protection.
- Quota surfaces, overrides, 20% default reserve, and five-minute maximum checkpoints.
- Machine-readable events, output bounds, timeout, cancellation, and run manifests.
- Risk-based approval for one representative Class 2/3 action.
- One independent verification command in a fresh sandbox.
- Local notification abstraction with a fake test adapter.

### Explicitly deferred

- Full daemon idle scheduling, retries/circuit breakers, and broad crash matrix beyond the vertical-slice recovery points (Phase 2).
- CodexBar/Tokscale real quota adapters (Phase 3); fake/manual surfaces establish the contract first.
- OpenAI and Anthropic real API calls (Phase 4); provider and budget contracts/default-deny tests begin earlier.
- Tailscale/SSH remote approvals, MCP server hosting, skill installation, and richer ACP controls (Phase 4).
- TUI/web UI and automatic update activation (Phase 5). Update contract and manual policy are already designed.

## Test suites

### Unit

- task schema, transitions, idempotency, dependency cycles;
- layered policy resolution/provenance and deny precedence;
- approval action-digest binding, expiry, replay, denial, and consumption;
- quota surface arithmetic, unknown/stale states, overrides, reserves, reset scheduling, and budget reservation;
- route hard filters and deterministic score tie-breaking;
- path canonicalisation, argv construction, redaction, output bounds, and event digest chain;
- projection render/import conflict handling;
- adapter fixture parsing for supported/unsupported versions.

### Integration

- SQLite concurrent writers, crash/reopen, lease expiry, backup/migration/restore;
- temporary Git repositories: dirty trees, conflicts, nested repos, explicit submodules, and multi-repo manifests;
- fake process trees: timeout, TERM/KILL escalation, child cleanup, partial/malformed JSONL, ANSI/PTY output;
- fake sandboxes: mount/network/secret/resource attestation mismatch;
- Docker backend conformance when Docker is available;
- fake AoE HTTP service: auth, session create/send/status/output, retries, schema drift, and cancellation mapping;
- malicious repository/tool output, symlink traversal, shell injection, and secret canaries.

### End-to-end

- the vertical-slice scenario above;
- crash injection after each transition and before/after each external-action claim;
- quota changed during a run: continue, shorten checkpoint, checkpoint/pause, and emergency stop outcomes;
- verifier failure after agent success;
- user-managed Git policy for this repository denies branch/commit actions.

## Cross-platform proof plan

| Platform | Phase 1 | Before general support claim |
| --- | --- | --- |
| macOS arm64 | Required local Rust tests; Docker backend; agent probes | Apple Container conformance; signed packaging; filesystem/keychain tests |
| Linux amd64/arm64 | CI build/unit/integration with fake backends | rootless Podman and Docker conformance; SELinux fixture where possible |
| WSL2 | CI compile/test where runner available or documented reproducible VM | Docker/Podman selection, Windows-mounted path denial/default, signal/process cleanup, path/permission tests |

Unsupported or unavailable runtimes must produce a healthy diagnostic with actionable evidence, not a panic.

## Security gates

- No test requires real subscription credentials in CI.
- Canary secret does not appear in database, events, stdout/stderr, patch, summary, or support bundle.
- Prompt/task/path metacharacters never change argv boundaries.
- Sandbox socket/home/SSH mount attempts fail before launch.
- Current-run policy tampering cannot change the effective policy hash.
- Agent self-report cannot create a verification pass.
- Output, retries, processes, runtime, and disk use remain bounded.

## Real smoke tests

Real tests are opt-in and individually labelled:

- `real-codex`: `scripts/test-real-codex-subscription-smoke` runs exactly one acknowledged fixture-repository patch task using ephemeral JSONL and read-only Codex permissions; it must prove exact scope, detached verification, unchanged source checkout, redacted artifacts, and no automatic retry;
- `real-claude`: bounded print/stream-JSON task with explicit tool and permission policy;
- `real-antigravity`: bounded print task with explicit timeout and captured text;
- `real-aoe`: authenticated loopback session lifecycle against a pinned supported AoE;
- `real-docker`, `real-podman`, `real-apple-container`: backend attestation and cleanup.

Before starting, each test presents provider/profile, possible quota or API cost, network, credentials, runtime, timeout, files, and cleanup. It never runs automatically in normal CI.

## Phase 1 exit report

The report must contain:

- exact implemented scope and deferred gaps;
- migration and schema versions;
- tested platform/runtime/agent versions;
- complete test command summary and failures/waivers;
- demo artifacts and reproducible steps;
- security attestation results and residual risks;
- AoE compatibility range and any hardened-profile limitations;
- no claim of production readiness;
- no commit, branch, push, PR, or merge unless the user separately authorises it.
