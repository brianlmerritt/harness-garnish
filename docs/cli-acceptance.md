# Harness Garnish CLI acceptance plan

Status: living, cross-phase acceptance plan for the project-supervisor CLI defined by the [CLI Bible](cli-bible.md), [ADR 0016](decisions/0016-project-supervisor-cli.md), and the frozen [`garnish.cli/v1alpha1` contract](tb0-command-contract.md). Update this document with every material normal-CLI change. Phase acceptance documents and exit reports remain dated evidence snapshots; this document is the continuing product-level ledger.

## 1. Purpose and authority

The CLI succeeds when an operator can supervise projects without manually coordinating Garnish's internal tasks, route pins, request plans, scheduler instances, worktrees, handoffs, or cleanup. A collection of individually working commands is not sufficient acceptance evidence.

The governing order is:

1. the CLI Bible and accepted ADRs define product intent and security boundaries;
2. the TB-0 command and state contracts define the versioned grammar and machine semantics;
3. this document defines the continuing end-to-end CLI acceptance obligations;
4. phase acceptance plans select the rows required for a phase and name exact machine tests;
5. exit reports record dated commands, platforms, results, waivers, and residual scope.

If implementation or this plan conflicts with a higher document, stop and resolve the design conflict before changing runtime behaviour. A product decision requires an ADR amendment or superseding ADR, not a silent change to an acceptance row.

## 2. Evidence rules

- Operator claims require black-box execution of the compiled `garnish` binary against temporary state. Library-only tests may support a claim but cannot establish the CLI boundary by themselves.
- Project and Git lifecycle claims use disposable, real Git repositories and compare the source checkout, `HEAD`, status, worktrees, and owned branches before and after the journey.
- Every normal command must cover success, invalid input, denied or unavailable operation where applicable, and atomic failure with no partial durable or external effect.
- Human and JSON output are separate interfaces. JSON schema, stdout/stderr separation, and exit codes are asserted exactly. Human output is asserted by required information and actionable wording rather than terminal decoration.
- Material mutations prove `--dry-run` has no database, filesystem, process, provider, notification, secret-store, or external effect. Naturally repeatable commands also prove idempotency.
- Status-like commands return success when they successfully describe an unhealthy state; tests inspect documented status and blocker fields.
- Normal acceptance removes provider credentials and all live-test acknowledgements. It uses fake agents, quotas, approvals, notifications, secrets, and execution backends and consumes no provider subscription quota or API budget.
- Real provider, container, keyring, desktop-notification, and packaging checks are separately labelled, bounded, non-retrying where an external effect is uncertain, and run only after explicit operator opt-in.
- No secret value, private chain-of-thought, raw provider body, or unbounded agent output may appear in argv, SQLite, stdout/stderr, logs, events, notifications, patches, handoffs, review artifacts, backups, or support material.
- A support claim is limited to platforms and runtime boundaries for which the same required acceptance set has machine evidence.

Manual exercises in the [CLI user acceptance guide](cli-user-acceptance.md) and the more narrowly scoped [TB-1 CLI testing notes](tb1-cli-testing.md) are useful exploratory evidence, but they do not replace automated assertions.

## 3. Continuous update procedure

Before a CLI implementation phase:

1. compare the proposed operator journey with the CLI Bible, command contract, and state contract;
2. add or revise the affected rows below, leaving their evidence state as `contract only` or `partial`;
3. name the intended black-box test and quota/external-effect boundary in the phase acceptance plan;
4. resolve any command, policy, or product-semantic disagreement before implementation.

During implementation, add the black-box failing test before or with the behaviour and keep unrelated compatibility behaviour unchanged. Advanced aliases do not count as normal-interface evidence.

After implementation:

1. run the phase acceptance wrapper with provider/live variables removed;
2. update the affected rows to `machine evidence` only when the named assertions pass;
3. record the exact command, date, platform, totals, ignored/live tests, and limitations in the phase exit report;
4. re-check the delivered journey against the CLI Bible and record any residual product drift as a blocker or planned row;
5. update this plan when a regression, newly supported platform, or later phase changes the evidence boundary.

Evidence states used below:

| State | Meaning |
| --- | --- |
| `machine evidence` | The named automated CLI evidence passes for the stated fake or real boundary. |
| `partial` | Some behaviour has machine evidence, but the full row is not accepted. |
| `contract only` | Grammar or intended behaviour is frozen, but runtime evidence is absent. |
| `deferred` | The governing product plan assigns the row to a later phase. |

## 4. Product journey acceptance matrix

This matrix is cumulative. A later phase must preserve earlier machine evidence as well as satisfy its newly selected rows.

| ID | Operator outcome and required assertions | Current evidence | Next acceptance boundary |
| --- | --- | --- | --- |
| CLI-J01 | Initialise private native state with safe defaults; report exact locations; repeated init is safe; `doctor` reports actionable healthy, degraded, and unavailable capabilities without submitting agent work. | `partial`: TB-1 proves temporary initialisation with `WWWOOBB`; the complete init/doctor journey is not covered. | Select and automate before claiming bootstrap readiness. |
| CLI-J02 | Configure `W`/`O`/`B` calendars, timezone, dated exceptions, and project affinity; previews explain eligibility and next wake without task IDs. | `partial`: `calendar_preview_applies_work_non_work_and_shared_days` proves the TB-1 weekly pattern, affinities, preview, and exception subset. | Preserve through later schemas; add invalid timezone/pattern, revision, dry-run, and next-wake cases. |
| CLI-J03 | Register work and non-work projects, add local objectives with acceptance criteria, and start/pause/resume/stop supervision without exposing an internal task ID. Invalid lifecycle transitions are atomic. | `partial`: TB-1 black-box tests prove the principal happy path and stop/pause/resume transitions; the complete negative and dry-run matrix remains open. | Complete command-state and error coverage as the lifecycle surface stabilises. |
| CLI-J04 | Start several projects and let the service choose only eligible ready work continuously, without manual claims, route pins, daemon names, or cleanup commands. Restart does not duplicate work or an uncertain external action. | `partial`: `bounded_service_cycle_selects_the_eligible_project_without_task_ids` proves deterministic one-cycle fake selection. Foreground continuity, service lifecycle, and crash recovery are not accepted here. | TB-1 behaviour must remain; durable service/recovery evidence is required before release readiness. |
| CLI-J05 | One `garnish status` or `project status` response explains project states, active work, selected route and reason, all quota surfaces, reserves/reset/forecast, approvals, important notifications, container health, handoffs/failures/verification/cleanup, paid fallback state, next action, and next wake. | `partial`: TB-1 project status proves objectives, review, next action, and cleanup fields. It does not yet provide the complete Bible status answer. | Expand cumulatively in TB-2 through TB-5; assert human and JSON parity. |
| CLI-J06 | Garnish hard-filters and explains routes, prefers eligible subscription agents, and checkpoints/hands off on low quota. Low subscription capacity never authorises paid API use. | `partial`: `project_first_fixture_routes_hands_off_reviews_and_cleans_without_task_commands` proves fake Codex rejection, fake Claude selection, handoff evidence, and no paid fallback. It is not a real checkpoint or subscription run. | TB-4 real subscription switching; TB-5 separately proves explicit, budgeted API fallback. |
| CLI-J07 | Contained low-risk work proceeds autonomously only inside an independently attested secure container; boundary-crossing actions cannot be self-authorised by an agent. Cancellation terminates descendants and cleanup is durable. | `deferred`: TB-3. Historical task-oriented container evidence is not evidence for this project-supervisor journey. | Fake conformance on each platform plus separately opted-in minimal live Codex evidence. |
| CLI-J08 | One structured elevated action creates exactly one bounded approval and notification; allow/deny binds to the displayed digest, expires, is single-use and replay-safe, and resumes or checkpoints safely. Acknowledging a notification never grants approval. | `deferred`: TB-2. Existing task-oriented approval/notification commands do not satisfy the target broker. | Fake structured request, outbox/inbox/delivery, expiry, replay, and resume/restart evidence. |
| CLI-J09 | Secret commands never accept values through ordinary argv; protected-store metadata, rotation, scope, health, projection, destruction, and canary redaction pass without revealing a value. | `deferred`: management foundation in TB-2 and agent projection in TB-3/TB-4. | Fake protected store first; separately opted-in platform keyring checks where required. |
| CLI-J10 | Successful work produces independently verified review evidence outside the source checkout; explicit apply/discard is base- and scope-bound and never commits, pushes, merges, opens a PR, or deploys. | `partial`: `project_first_fixture_routes_hands_off_reviews_and_cleans_without_task_commands` proves TB-1 fixture review/apply, unchanged source `HEAD`, and cleanup; discard and negative boundaries remain open. | Add conflict, discard, replay, dirty checkout, stale base, symlink/path, and dry-run cases. |
| CLI-J11 | Terminal success and failure remove exactly owned implementation/verifier worktrees and branches after durable evidence; ambiguity quarantines and notifies rather than deleting. Maintenance preview and cleanup are digest-bound and idempotent. | `partial`: TB-1 proves exact owned cleanup after success and an escaping-write failure. Quarantine, immutable cleanup plan, reconcile, retention, and notification remain open. | Extend through TB-2 and release-retention work. |
| CLI-J12 | Project stop and global emergency stop prevent new claims, checkpoint or terminate active work, preserve bounded evidence, and leave no owned execution running. | `partial`: TB-1 proves stopped project state after fixture completion. Active-run stop and global emergency-stop evidence are absent. | Add fake process-tree and crash-boundary evidence before real agents. |
| CLI-J13 | Routine help and operator documentation expose only the project-supervisor workflow. Internal tasks and compatibility controls use `advanced`; removing an alias requires replacement evidence and a deprecation window. | `partial`: TB-1 proves `advanced task list`, and legacy aliases are hidden but still accepted. | Assert top-level help, advanced help, alias warnings/rejection schedule, and documentation links each phase. |

## 5. Versioned CLI interface acceptance

The frozen contract is not considered implemented merely because its JSON fixture parses.

| ID | Interface requirement | Current evidence |
| --- | --- | --- |
| CLI-I01 | The 18 normal families and `advanced` gateway match `garnish.cli/v1alpha1`; normal help does not advertise legacy task-oriented families. | `partial`: TB-0 contract fixtures cover the to-be grammar; TB-1 runtime exposes the main slice and hides legacy aliases. Executable coverage of every family is incomplete. |
| CLI-I02 | Global `--data-dir`, `--output human|json`, `--no-color`, and `--quiet` obey the frozen grammar at every command depth. | `contract only`; the transitional executable does not implement the full global surface. |
| CLI-I03 | Every JSON success uses its required versioned envelope and resource fields; exactly one object is written to stdout. Unknown additive fields remain safe for consumers. | `contract only` overall; TB-1 intentionally emits transitional JSON shapes. |
| CLI-I04 | Every error writes no stdout, emits the versioned error object on stderr in JSON mode, and returns stable exit code 2 through 9 for usage, validation, not found, conflict, denied, unavailable, external uncertainty, or internal failure. | `contract only`; the transitional executable currently collapses runtime failures to exit code 1. |
| CLI-I05 | Human output is the interactive default and gives concise results, reasons, next safe action, next wake, and exact corrective commands without requiring JSON parsing. | `contract only`; TB-1 output remains JSON-only. |
| CLI-I06 | Every material command implements effect-free `--dry-run`; applicable repeat calls are idempotent; ambiguous names return bounded non-secret candidates. | `contract only` except for state-specific idempotency already covered by historical internals. |
| CLI-I07 | Help, examples, and errors use project vocabulary; every copyable documentation command declares placeholders immediately before its block. | `partial`: `scripts/check-command-placeholders` passes the TB-0/TB-1 documentation; complete human help/error review remains open. |

## 6. Normal-family coverage ledger

Each normal family remains unaccepted until its relevant journey and interface rows both have machine evidence.

| Family | Current accepted boundary | Planned phase or gap |
| --- | --- | --- |
| `init` | TB-1 fixture defaults only | CLI-J01 and full interface contract |
| `doctor` | No project-supervisor acceptance | Bootstrap and platform capability matrix |
| `status` | TB-1 project/objective/review/cleanup subset | CLI-J05 cumulatively through TB-5 |
| `service` | TB-1 bounded fake cycle selection; the foreground form is implemented but lacks named black-box acceptance | Foreground lifecycle, installation/control/recovery, and release work |
| `config` | TB-1 revisioned calendar-pattern explanation | Setting mutation negatives and complete layering/provenance in TB-2 |
| `calendar` | TB-1 weekly pattern, affinities, previews, and exceptions | Negative, dry-run, revision, and next-wake coverage |
| `project` | TB-1 add, start/pause/resume/stop, status, review/apply, and automatic cleanup; other forms are implemented but lack named black-box acceptance | List/show/configure/discard/archive/remove plus full state/error/dry-run contract |
| `objective` | TB-1 local add and deterministic internal-task creation observed through project status and `advanced` | List/show/complete/cancel, negative cases, and dependency contract |
| `agent` | Two immutable fake profiles and fake quota configuration | Registration/probes, then real Codex/Claude profiles |
| `quota` | Fake routing evidence uses profile quota; historical low-level quota machinery remains | Target refresh/status/explain/override interface |
| `route` | Runtime explanation exists but lacks named black-box acceptance | Full hard-filter/score/status explanation |
| `approval` | No target project-supervisor acceptance | TB-2 |
| `notification` | No target outbox/delivery acceptance | TB-2 |
| `secret` | Not implemented on the normal surface | TB-2 and projection phases |
| `policy` | Not implemented on the normal surface | TB-2 |
| `events` | Not implemented on the normal surface | Add with the applicable control-plane phase |
| `ops` | Historical operations exist; no target emergency-stop journey evidence | CLI-J12 and release operations |
| `maintenance` | TB-1 proves automatic owned cleanup; preview/reconcile forms are implemented but lack named black-box acceptance | Digest-bound plan, quarantine, retention, integrity |

## 7. Mandatory negative and resilience suites

Each affected phase must select and automate the applicable cases rather than relying only on happy-path demonstrations:

- malformed operands, options, dates, timezones, patterns, percentages, money, digests, paths, and JSON;
- missing, ambiguous, stale-version, wrong-owner, wrong-base, expired, replayed, and already-terminal resources;
- lifecycle transitions attempted from every invalid source state;
- dry-run snapshots proving byte-identical database and filesystem state and zero launched processes or external calls;
- dirty source checkout, changed `HEAD`, nested repositories, submodules, symlink traversal, escaping paths, hostile filenames, hooks, and unexpected worktrees/branches;
- quota unknown/stale/low/reset transitions, every hard route rejection, deterministic ties, handoff failure, and no eligible route;
- process timeout, cancellation, descendant cleanup, malformed/bounded output, crash after each material transition, restart, and uncertain external effect;
- sandbox attestation mismatches for mounts, user, network, credentials, resources, image/runtime, and cleanup;
- approval digest mismatch, expiry, duplicate delivery, allow/deny replay, notification acknowledgement, resume failure, and checkpoint restart;
- secret canaries across every durable and output surface, protected-file permissions, rotation invalidation, and projection destruction;
- concurrent commands, SQLite busy/reopen/integrity, interrupted backup, cleanup replay, and ownership ambiguity;
- human and JSON golden fields, stdout/stderr separation, exit codes, `--quiet`, `--no-color`, TTY/non-TTY use, and interrupt handling.

## 8. Platform and external evidence

Normal CI and local regression use quota-free fakes. Before claiming a platform-supported project-supervisor release, the applicable cumulative rows must pass on macOS arm64, Linux amd64/arm64, and WSL2, with unsupported capabilities reported as actionable degraded states.

Live sign-off is phase-specific:

| Boundary | Required opt-in evidence |
| --- | --- |
| TB-2 | Platform keyring and native desktop notification checks only where support is claimed; fake approval/secret/notification evidence remains the normal gate. |
| TB-3 | One acknowledged minimal live Codex project run per claimed secure-container/runtime boundary, with no automatic retry of uncertain subscription use. |
| TB-4 | Minimal live Claude and cross-agent checkpoint/handoff tests after fake end-to-end switching passes. |
| TB-5 | One-request paid-provider checks only for explicitly enabled, hard-budgeted API routes; fake exhaustion and default-deny remain mandatory. |
| TB-6 | Clean-install packaged acceptance, service install/upgrade/rollback, backup/restore, multi-project soak/recovery, and UI parity on every claimed platform. |

The user must opt in before any provider-quota, paid-API, real-keyring, real-notification, or real-container test that is not already an explicitly selected phase boundary.

## 9. Current repeatable baseline

The current cumulative runtime wrapper is the TB-1 quota-free acceptance command. It includes formatting, the focused project-supervisor CLI suite, schema-21 migration, strict Clippy, and the full workspace regression. The TB-0 contract tests run again inside that workspace regression.

Placeholders: none.

```console
./scripts/test-tb1-cli
```

The latest dated result belongs in [the TB-1 exit report](tb1-exit-report.md), not as an evergreen claim in this plan. When TB-2 begins, its acceptance plan must name a new cumulative wrapper or explicitly extend this one; the command above must not silently acquire live external effects.

## 10. Project-supervisor MVP exit gate

The project-supervisor MVP may be called complete only when all 13 outcomes in section 17 of the CLI Bible are mapped to `machine evidence` rows here, every required normal-family interface is implemented, cumulative quota-free acceptance passes on each claimed platform, and the separately labelled live tests required by the claimed Codex/Claude/runtime boundaries have explicit operator opt-in and recorded results.

An exit report must still state residual limitations. Passing a phase wrapper, a manual CLI exercise, historical task-oriented evidence, or a contract-parser test alone is never sufficient to claim the project-supervisor MVP.
