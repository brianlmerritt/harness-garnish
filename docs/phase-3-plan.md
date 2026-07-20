# Phase 3 plan — multi-agent and quota routing

## Objective

Phase 3 replaces single-candidate routing with deterministic selection across Codex CLI, Claude Code, and Antigravity. Hard policy, capability, health, freshness, quota, reservation, and manual-pin gates remain outside every agent. Scoring may rank only candidates that pass all hard gates, and every component of the selected score is recorded.

Phase 3 exited successfully on 2026-07-20. Final macOS, Linux VPS, and WSL2 evidence is recorded in [`phase-3-exit-report.md`](phase-3-exit-report.md).

Normal tests use fake executables and quota fixtures. Real CLI and quota-provider checks are separate opt-ins and must state whether they can consume subscription quota before running.

## Implementation stages

1. Durable capability matrix: persist versioned agent probe evidence with expiry; expose fresh, stale, unsupported, missing, and unknown states without invoking an agent task.
2. Pure multi-candidate router: capability and policy filters, deterministic score components/tie-breaking, manual pinning, and complete candidate rationale.
3. Quota providers: a versioned provider contract, CodexBar machine-readable parser, historical-usage observations, freshness/confidence, and fail-closed parser drift.
4. Reservations: atomically reserve forecast headroom across every relevant subscription surface and release/reconcile it on completion, cancellation, failure, or recovery.
5. Scheduler integration: select and claim the winning adapter/account together, preserve route-specific ceilings, and re-route safely after live quota or health changes.
6. Operator interface: an authenticated loopback-only dashboard over canonical projects, queue reasons, agent/quota state, approvals, and activity; mutations remain disabled until they have CLI-equivalent policy and evidence.
7. Independent verification: select a verifier separately from the implementer when policy requires it and retain distinct run/evidence records.

## Machine acceptance

| ID | Assertion |
| --- | --- |
| P3-01 | Codex, Claude, and Antigravity probes persist executable, version, capabilities, health, failure detail, observation time, and expiry without consuming task quota. |
| P3-02 | Missing, unsupported, unknown, and stale probes fail their relevant hard gates with stable reason codes; version/help/output drift fixtures fail closed. |
| P3-03 | Candidate ordering is deterministic across restart and records every hard-filter and score component; lexical identity is the final tie-breaker. |
| P3-04 | A durable manual pin selects only the exact allowed adapter/account and never bypasses capability, policy, health, freshness, or quota gates. |
| P3-05 | Unknown, stale, malformed, and conflicting quota observations remain distinct and never become invented available headroom. |
| P3-06 | Five-hour, weekly, monthly, and paid-extra surfaces retain independent user-adjustable percentages/reserves and reset times during routing. |
| P3-07 | Concurrent schedulers cannot over-reserve any quota surface; recovery releases an orphaned reservation once. |
| P3-08 | A mid-project quota override or manual pin change causes deterministic re-evaluation without rewriting the original decision evidence. |
| P3-09 | Independent verification uses a separately selected run and clean verification worktree; policy can require a different adapter/provider. |
| P3-10 | Normal macOS/Linux/WSL2 suites use fake providers only and make no provider subscription or paid API calls. |
| P3-11 | The local operator interface binds only to loopback, requires an ephemeral token/cookie, rejects invalid hosts and unauthenticated state reads, and renders durable reason codes without enabling mutations. |
| P3-12 | Historical forecasts consume only explicit deduplicated evidence, remain exact-identity scoped and restart durable, use bounded conservative P90 after five groups, and otherwise retain the fallback. |

## First slice

Schema 8 introduces append-only agent capability probes. `agent refresh` records Codex, Claude, and Antigravity probe evidence with a bounded validity interval; `agent status` reports the latest matrix as fresh, stale, or unknown. The next slice will make the multi-candidate routing kernel consume this evidence.

The first real, quota-free schema-8 refresh passed on macOS and persisted healthy evidence for Codex CLI `0.144.6`, Claude Code `2.1.215`, and Antigravity `1.1.4`. Their installed help surfaces confirm the pinned headless, structured-output/tool-policy, resume, sandbox, and timeout interfaces described by the adapter contract. The checks invoked only `--version` and `--help`; no agent task or provider request was submitted.

The pure multi-candidate routing kernel is also implemented. It distinguishes unknown/stale probe evidence, every adapter health state, missing capabilities, unknown quota, insufficient quota, and manual-pin mismatches before scoring. An exact pin excludes other identities but cannot bypass a hard gate. Eligible candidates score as `quota margin + 0.25 × historical success + 10 continuity bonus + policy preference`; unknown historical success uses a neutral 50%. Results sort by eligibility, descending score, then lexical adapter/provider/account identity, so input order cannot change the winner.

Schema 9 persists exact task pins as an adapter/provider/account triple and records pin/unpin events with an operator reason. Scheduler tick and daemon configuration accept repeatable `ADAPTER:PROVIDER:ACCOUNT` candidates, preflight every candidate, apply the pure score, persist the complete candidate matrix, and atomically claim capacity for the selected identity. A missing configured pin records `manual_pin.unavailable`; a pinned candidate that fails health, capability, policy, or quota remains denied.

Schema 11 implements the current CodexBar machine JSON contract with bounded argv-only execution, object/array parsing, recognized JSONL diagnostic preludes, known-window normalization, additive-field tolerance, structural and numeric drift rejection, append-only observations, source confidence, five-minute default validity, and raw-payload digests. Expected five-hour or weekly lanes that CodexBar omits are persisted as unknown evidence rather than silently disappearing. Provider failures extract their bounded structured message whether CodexBar writes it to stdout or stderr. Stale evidence fails routing and mid-run checkpoints unless a live explicit override applies. Scheduler claims atomically sum and reserve forecast percentage across every selected account surface; a two-connection race admits only the claim that fits, and expiry recovery releases the orphaned reservation exactly once. P3-05 and P3-07 now have deterministic quota-free coverage. A real Codex OAuth refresh through CodexBar `0.45.2` passed on macOS on 2026-07-20 and exposed an unavailable expected lane, strengthening the missing-window fixture. A real Claude CLI refresh through Claude Code `2.1.215` also passed after Garnish preserved the narrowly allowlisted terminal, identity, locale, temporary-directory, configuration, and tool-manager context needed by CodexBar's nested CLI probe. API keys, OAuth tokens, and the user's private quota values remain excluded from the collector environment and project documentation.

Schema 12 makes every explicit collector success or failure durable in `quota_collection_attempts`; `quota attempts` exposes the bounded evidence. A failed refresh does not overwrite the last successful observation, which naturally becomes stale when its validity ends.

The first operator-interface slice adds Overview, Projects, Queue, Agents & quotas, Approvals, Activity, and Settings pages plus an authenticated `/api/v1/snapshot` endpoint. It is responsive, dependency-light, and intentionally read-only. The server binds to `127.0.0.1`, generates a new random token on every start, exchanges the startup query token for a strict `HttpOnly` cookie, validates the exact loopback `Host`, and sends no-store/CSP/frame-denial headers. Queue explanations consume durable scheduler wake reason codes. Deterministic HTML escaping, authentication, cookie, API, and projection tests use only local fixtures and no provider quota.

Schema 13 adds append-only, replay-detecting usage samples and exact adapter/provider/account forecasts. Account-level quota deltas are deliberately not treated as run consumption. At least five evidence groups are required before the bounded 50-group nearest-rank P90 replaces the conservative fallback; task uncertainty is then applied and the result is capped at 100%. Multi-candidate routing records each candidate's forecast source/sample count and scheduler admission reserves the selected candidate's value. P3-12 has deterministic restart, identity isolation, duplicate, fallback, P90, route-gate, and CLI JSON coverage.

Schema 14 makes verification a separate selected child run with its own route, manifest, clean detached worktree, bounded output, and verification artifact. Exact implementer identity is always excluded; default policy requires a different adapter and can additionally require a different provider. The first quota-free `garnish-command-verifier:local:default` executes only predeclared verification argv. P3-09 has deterministic separation-policy and end-to-end vertical-slice coverage. Agent-based semantic review remains a future adapter and is not implied by the command verifier.

## Explicit non-goals

Phase 3 does not enable paid OpenAI/Anthropic API agents, remote workers or approvals, automatic Git integration, Skills/MCP installation, or an automatic fallback from subscription CLIs to paid APIs.
