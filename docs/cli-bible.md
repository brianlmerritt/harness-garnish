# Harness Garnish CLI Bible

Status: authoritative **to-be product and operator contract** under [ADR 0016](decisions/0016-project-supervisor-cli.md). This document describes the intended Harness Garnish experience and phased route from the current implementation. A capability described here is not implemented merely because it is specified. Machine evidence in the applicable acceptance plan or exit report remains mandatory before any phase is called complete.

The archived narrow task-oriented CLI boundary is retained at [`archive/cli-mvp.md`](archive/cli-mvp.md). Its evidence remains valid for that compatibility slice, but it does not define the intended product interface.

## 1. Product purpose

Harness Garnish is a local, multi-project supervisor for quota-aware AI-assisted development. It observes subscription capacity, schedules eligible project work, routes work among coding-agent CLIs and explicitly enabled APIs, supervises agents in attested sandboxes, brokers approvals, verifies results, preserves handoffs, and tells the user when intervention is genuinely required.

The user manages projects, policies, accounts, schedules, budgets, approvals, and exceptional decisions. The user does **not** manually create, pin, prepare, route, execute, review, and clean up every internal task.

Internal tasks, claims, leases, route decisions, worktrees, runs, reservations, checkpoints, and handoffs remain valuable canonical records. They are implementation and evidence units rather than the normal operator interface.

## 2. Product invariants

1. Projects, not individual tasks, are the primary operator unit.
2. Garnish chooses among eligible agents using capability, health, subscription quota, project policy, continuity, reliability, schedule, and cost evidence.
3. Low subscription capacity causes a safe checkpoint and structured handoff to another allowed system; it does not require the user to rebuild the work item manually.
4. Paid API use is never inferred from low subscription quota. It requires durable project enablement, a hard budget, protected credentials, and an applicable approval policy.
5. Low-risk reads, writes, builds, and tests may proceed autonomously only inside an independently attested secure container and within effective project policy.
6. Agents propose actions but never authorise themselves, change their own governing policy, or mark their own work verified.
7. Human approvals are exact, bounded, expiring, replay-resistant, and bound to the canonical action presented to the user.
8. Notifications report durable control-plane events. They are not themselves approval authority.
9. Secret values never enter command arguments, SQLite, project configuration, prompts, logs, patches, projections, or support artifacts.
10. Settings are layered, versioned, validated, and explainable field by field.
11. Calendar eligibility is resolved from the user's system calendar and the project's affinity, not recreated on each internal task.
12. Failed or completed execution environments are cleaned according to explicit retention policy; live Git worktree and branch clutter is not an acceptable default.
13. Normal and portability tests consume no provider subscription quota or API credit. Every live test remains separately labelled and explicitly opted into.
14. Every copyable command in operator documentation declares all placeholders immediately before the command block. Literal acknowledgement values are identified as literals, not disguised placeholders.

## 3. Operator vocabulary

| Term | Meaning |
| --- | --- |
| System | The local Garnish installation, its user-wide settings, calendars, agent accounts, secret providers, notification channels, and scheduler service. |
| Project | A registered repository or explicitly declared multi-repository unit with a goal/backlog, schedule affinity, routing preferences, autonomy policy, verification rules, and optional API budget. |
| Objective | User-visible project work at a useful outcome level. An objective may be decomposed into internal tasks by deterministic rules or an authorised planning agent. |
| Internal task | A durable scheduling/evidence unit. Normally hidden from routine operation but inspectable through advanced diagnostics. |
| Agent profile | A versioned Codex, Claude Code, Antigravity, API, or future-agent configuration bound to an account, executable, model policy, capability probe, quota source, and sandbox requirements. |
| Route | A particular adapter/provider/account choice that has passed every hard filter. |
| Run | One supervised execution attempt against owned project state. |
| Approval | A durable allow or deny decision for one canonical external or elevated action. |
| Notification | An alert derived from a durable event, optionally delivered through several channels. |
| Secret reference | A non-secret locator for a value held by an OS keyring, protected file, environment provider, or approved secret service. |

## 4. Normal operator journeys

### 4.1 First-time setup

The setup journey must:

- initialise private state and configuration locations;
- probe Git, supported container runtimes, Agent of Empires or the selected execution plane, agent CLIs, quota collectors, OS notification support, and secret-store support;
- let the user register existing Codex and Claude CLI profiles without copying authentication into Garnish state;
- configure the system calendar, default quota reserves, notification channels, execution backend order, and retention policy;
- explain degraded operation where a secure container or protected credential projection is unavailable.

The target command families are `garnish init`, `garnish doctor`, `garnish config`, `garnish calendar`, `garnish agent`, `garnish secret`, `garnish notification`, and `garnish service`.

### 4.2 Register and configure a project

Project registration must require only the repository identity and user-visible project intent. Project configuration then defines:

- title, root path, objective/backlog source, priority, and dependencies on other projects;
- calendar and project affinity;
- preferred subscription agents and allowed fallbacks;
- autonomy, Git, network, dependency-installation, MCP/skill, and external-effect policy;
- verification commands or discovery policy;
- checkpoint, retry, retention, and failure policy;
- optional paid API accounts, models, roles, tools, and hard budgets;
- notification routing and escalation preferences.

The target operator family is `garnish project add|list|show|configure|start|pause|resume|stop|status`.

### 4.3 Start and supervise project work

`project start` makes a project eligible for the scheduler. Garnish selects or creates internal work, calculates the ready set, chooses an allowed route, starts an attested execution environment, supervises checkpoints, and records evidence. The operator does not manually copy task IDs, create route pins, issue readiness probes, construct request plans, name daemon instances, or clean Garnish-owned worktrees.

`garnish status` must answer, without requiring several diagnostic commands:

- which projects are running, waiting, paused, blocked, complete, or unhealthy;
- what each active agent is doing and where;
- which account/model/route is active and why it was selected;
- current subscription surfaces, reserves, forecasts, and reset times;
- pending approvals and unread important notifications;
- the next scheduled project/action and next wake time;
- current container/backend health;
- recent handoffs, failures, verification results, and cleanup state;
- paid-budget availability and whether API fallback is enabled.

### 4.4 Intervene only when necessary

The ordinary intervention commands are project pause/resume/stop, approval allow/deny, quota override, emergency stop, and review/integration decisions required by project policy. Internal task commands remain available under an advanced or diagnostic surface for support and recovery.

## 5. Target command system

The command names below are a product contract to refine before implementation. Angle-bracketed names in this section describe typed operands; they are not copyable shell blocks.

| Family | Required responsibilities |
| --- | --- |
| `garnish init` | Create private state/config, select safe defaults, and report exact locations. |
| `garnish doctor` | Probe runtimes, agents, quota collectors, keyrings, notifications, execution plane, permissions, and compatibility without submitting agent work. |
| `garnish status` | Present the complete multi-project operational summary and actionable blockers. |
| `garnish service` | Install where supported, run, start, stop, restart, and inspect the local scheduler/control service. |
| `garnish config` | Show, edit, set, validate, export non-secret settings, and explain effective values and provenance. |
| `garnish calendar` | Configure `W`/`O`/`B` patterns, timezones, dated exceptions, preview eligibility, and assign calendars. |
| `garnish project` | Register, configure, start, pause, resume, stop, inspect, archive, and remove projects under safe lifecycle rules. |
| `garnish objective` | Add or inspect high-level project outcomes when the configured backlog source permits direct local objectives. |
| `garnish agent` | Register CLI/account profiles, probe versions/capabilities, inspect authentication readiness, select models, and configure quota sources. |
| `garnish quota` | Refresh and explain all subscription surfaces, reserves, forecasts, overrides, reservations, and reset/wake decisions. |
| `garnish route` | Explain candidate selection and project fallback policy; manual pins are advanced exceptions, not routine workflow. |
| `garnish approval` | List, show, allow, deny, and audit exact pending actions. |
| `garnish notification` | List, acknowledge, configure, test, mute, and inspect delivery attempts without changing approval state. |
| `garnish secret` | Add, reference, test, rotate, list metadata, and remove secrets through supported protected stores. |
| `garnish policy` | Show, validate, explain, and revise global, agent/account, and project policy within managed ceilings. |
| `garnish events` | Inspect bounded durable project, quota, approval, notification, execution, and recovery history. |
| `garnish ops` | Pause all new work, resume, perform emergency stop, diagnostics, backup, restore checks, and health inspection. |
| `garnish maintenance` | Preview and perform safe cleanup, retention, database integrity, worktree reconciliation, and artifact garbage collection. |
| `garnish advanced` | Expose internal tasks, claims, runs, pins, reservations, raw readiness, and compatibility diagnostics for support and development. |

Human-readable output is the interactive default. Every mutating command also has stable machine-readable JSON, predictable exit codes, idempotency where applicable, and a dry-run/preview mode when the effect can be material.

## 6. Settings and effective policy

### 6.1 Precedence

Settings and policy resolve from strongest to weakest:

1. managed or organisation constraint;
2. global user setting/policy;
3. named agent/account profile;
4. project setting/policy;
5. internal task override;
6. current run request.

Denies win. Security and budget ceilings resolve to the most restrictive applicable value unless a field is explicitly defined as additive. A lower layer may narrow freely but may widen only within a delegable ceiling declared by the higher layer.

### 6.2 Storage and trust

- Bootstrap and non-secret host preferences live in a versioned, validated native configuration file.
- Operational project, routing, quota, approval, and policy revisions are canonical transactional state.
- Repository-local configuration is agent-readable input and may suggest changes, but it cannot silently modify the policy governing that repository or current run.
- Secret values never live in configuration or canonical state.
- Unknown security-relevant keys are errors rather than ignored suggestions.

`config explain <path>` must show the effective value, every contributing source/revision, overridden values, managed ceiling, delegability, and whether a restart or reschedule is required.

### 6.3 Settings domains

The system must provide explicit settings for:

- native data/config/cache locations and retention;
- calendars, timezone, day-boundary behaviour, and optional working windows;
- execution-plane and sandbox backend order;
- default resource/checkpoint/retry ceilings;
- agent executables, accounts, models, quota collectors, and routing preference;
- global and per-surface quota reserves;
- project affinity, priority, concurrency, backlog/planning, Git, network, verification, and integration policy;
- API enablement, budget periods, allowed models/tools/roles, concurrency, and fallback policy;
- approval expiry and effect-class defaults;
- notification channels, severities, quiet hours, and escalation;
- secret-store backend and projection policy;
- update channel and activation policy;
- cleanup and evidence retention.

## 7. Calendar model: `WWWOOBB`

### 7.1 Day classes

System calendar patterns are seven characters, Monday through Sunday:

- `W`: work-project day;
- `O`: off/non-work-project day;
- `B`: both work and non-work projects may run.

The user's required default pattern is `WWWOOBB`.

### 7.2 Project affinity

Projects declare affinity once and internal work inherits it:

| Project affinity | Eligible system day classes |
| --- | --- |
| `work` | `W`, `B` |
| `non-work` | `O`, `B` |
| `both` | `W`, `O`, `B` |

Therefore a work project under `WWWOOBB` is eligible on the three `W` days and the two `B` days, and ineligible on the two `O` days.

Internal work may narrow project eligibility for an exceptional reason, but it may not widen beyond project/system policy without an explicit higher-authority change.

### 7.3 Time and exceptions

- Every calendar uses an IANA timezone.
- Dated exceptions may classify a day as `W`, `O`, or `B` and record a reason.
- Optional working windows are separate from day class and must never be invented from locale.
- A queued project becoming ineligible receives an explained next wake time.
- An active project crossing a boundary checkpoints and pauses at the next safe boundary; it is not killed merely because the date changed.
- Calendar revisions immediately re-evaluate queued projects and are pinned in route/run evidence.

The current `W`/`O` calendar plus task-level `B` affinity is a migration source, not the target model.

## 8. Projects, objectives, and internal work

A project is the user's durable management unit. It owns its calendar affinity, objectives/backlog, policies, agent order, budgets, verification defaults, and lifecycle.

The backlog source must be explicit and may be one of:

- objectives stored in Garnish;
- an explicitly imported project backlog projection;
- an approved issue-provider integration;
- an authorised planning agent that decomposes a project objective;
- a bounded combination with deterministic precedence and conflict reporting.

Planning output is untrusted until validated. It may create proposed internal tasks but cannot change project policy, budgets, credentials, approval rules, or acceptance authority.

Internal tasks remain necessary for dependency graphs, claims, retries, handoffs, evidence, and verification. Normal operators see them through project status and objectives. Advanced users may inspect them directly without being required to operate them one by one.

## 9. Agent profiles, quota, routing, and handoff

Each agent profile binds:

- adapter and provider identity;
- account label and authentication readiness;
- exact executable and supported version range;
- capabilities and freshness of the latest probe;
- model selection/default policy;
- structured event and approval capabilities;
- subscription quota collectors and all applicable surfaces;
- sandbox/container requirements;
- task-scoped credential projection method;
- continuity, reliability, latency, cost, and project preferences.

The router first applies hard capability, platform, health, calendar, policy, secret, network, sandbox, quota, concurrency, and budget gates. It then scores eligible routes. Every selection and rejection is explainable.

At a safe checkpoint, new quota evidence may:

- renew the current route;
- shorten the next checkpoint;
- checkpoint and pause until reset;
- checkpoint and hand off to another eligible subscription CLI;
- use an enabled paid API route only when the project explicitly allows quota-triggered API fallback and the complete budget/approval boundary passes.

A cross-agent handoff contains goal, acceptance criteria, repository/base state, current diff/commit, command and verification results, decisions, assumptions, blockers, artifacts, unverified facts, and next safe action. It contains no private chain-of-thought and does not pretend vendor conversations are portable.

## 10. Secure execution and container authority

Routine autonomous development occurs in an independently attested container. `secure_container=true` requires verified effective properties, including:

- the owned project worktree as the only writable project mount;
- no host home, unrelated repository, container-engine socket, or uncontrolled local service mount;
- explicit non-root user and bounded CPU, memory, process, time, and output limits;
- explicit network state and destination allowlist;
- task-scoped, purpose-bound credential projection only where required;
- cancellation, process-tree termination, and cleanup evidence;
- inspected backend state matching the requested profile.

Inside that boundary, project-permitted reads, edits, builds, tests, and local temporary operations should not generate repetitive agent permission prompts. Codex or Claude may use a non-interactive/broad internal permission mode only after the outer sandbox attestation and effective policy pass.

Actions that cross the boundary—new network, secret use, host access, external messages, remote Git, deployment, destructive changes, persistent service creation, or high cost—remain controlled by Garnish policy and approval regardless of the agent's internal mode.

Subscription authentication projection is a first-class security design. Writable global Codex or Claude credential directories must not be mounted into task containers. The preferred boundary is a minimal, private, ephemeral, provider-specific projection that permits only the selected run and is destroyed afterward. A host-side fallback is allowed only with an honest degraded-isolation statement.

## 11. Approval broker

### 11.1 Decision flow

1. An adapter receives a structured tool/permission request.
2. Garnish canonicalises tool identity, argv/input, cwd, files, network destinations, secret references, external recipients, cost, project, run, and effect metadata.
3. Garnish computes an action digest and evaluates effective policy plus current sandbox attestation.
4. The decision is `allow`, `deny`, or `ask`.
5. `allow` continues only the exact action. `deny` returns a bounded explanation to the agent. `ask` transactionally creates a pending approval and notification and safely pauses the run.
6. An operator decision is single-use, expires, and must match the original digest before consumption.
7. The adapter resumes the same structured session when supported; otherwise Garnish restarts from a checkpoint/handoff without widening the action.

Routine Class 0/1 container actions should be pre-authorised. The operator queue is reserved for genuinely elevated or external effects.

### 11.2 Agent integration

- Claude Code should use its structured non-interactive stream, allowed/disallowed tools, and a narrowly scoped Garnish permission-prompt MCP tool for unresolved requests.
- Codex should use a versioned structured approval interface through ACP, app-server, or another proven adapter contract. Plain terminal prompt scraping and automatically typing confirmation text are not an acceptable unattended authority boundary.
- An adapter without structured approval/resume support may run only actions fully pre-authorised by the attested container profile; unexpected escalation fails or checkpoints rather than being guessed.

## 12. Notifications

Notifications are generated from canonical events in the same transaction or a transactional outbox. Initial event families include:

- approval required, decided, denied, or expired;
- project blocked, paused, failed, recovered, completed, or ready for review;
- quota low, unknown, stale, exhausted, reset, or overridden;
- route changed or handoff performed;
- all subscription routes unavailable;
- paid API fallback awaiting approval or nearing budget thresholds;
- credential missing, expired, rejected, or due for rotation;
- sandbox/execution-plane/backend unhealthy;
- verification failure or repeated retry exhaustion;
- cleanup failure, orphaned worktree, or state-integrity problem;
- emergency stop, security incident, backup failure, or update action.

Delivery is separate from authority. Initial channels are the durable local inbox, CLI/status, authenticated local web UI, and native desktop notifications. Email, messaging integrations, and SSH/Tailscale remote approval are later opt-in channels.

Notification settings define event/severity filters, quiet hours, grouping/deduplication, delivery retry, escalation, and acknowledgement. Secret values, raw credentials, private model reasoning, and unbounded agent output never enter notifications.

## 13. Secrets and credentials

### 13.1 Secret management contract

The `secret` family manages metadata and protected-store operations without accepting secret values as ordinary argv. Secret input uses a private TTY prompt, protected file descriptor, or approved external secret provider.

Supported target backends are:

- macOS Keychain;
- Linux Secret Service/keyring where available;
- protected user-owned mode-`0600` files for headless Linux/WSL2;
- short-lived environment injection for controlled automation;
- later, explicitly integrated external secret managers.

Canonical state stores only secret ID, backend/locator, provider/account, purpose, allowed projects/adapters/phases, creation/rotation/expiry metadata, and non-secret health evidence.

### 13.2 Delivery and lifecycle

- Resolve a secret only after route, policy, budget, approval, and sandbox gates pass.
- Deliver only to the selected adapter and phase through the narrowest supported mechanism.
- Prefer ephemeral tmpfs/private-file projection over inherited global environment.
- Redact known fingerprints and prevent values from implementing `Debug`, cloning, or serialization.
- Destroy ephemeral projections on completion, cancellation, failure, or recovery.
- Credential rotation invalidates affected probes/sessions and creates actionable notifications.
- Subscription CLI auth remains owned by the vendor CLI profile; Garnish records readiness and projection method rather than duplicating long-lived tokens.
- Git/deployment credentials are denied to coding containers by default and used only by separately authorised integration brokers.

## 14. Lifecycle, failure, cleanup, and retention

- Failures retain bounded events, process hashes/output according to redaction policy, route/quota evidence, sandbox attestation, checkpoint/handoff, verification evidence, and any safe patch artifact.
- A failed run does not silently retry an uncertain provider request or reuse an unsafe environment.
- Garnish automatically removes containers and detached verifier worktrees after terminal evidence is durable unless retention policy explicitly keeps them.
- Dirty failed implementation worktrees are either captured as bounded quarantine evidence or retained with an explicit expiry; they do not remain indefinitely without an alert.
- Garnish-owned branches are removed only after ownership, expected base, and absence of unexpected commits are verified.
- Cleanup is idempotent and source-checkout invariants are checked before and after.
- The maintenance preview explains every proposed deletion. Material evidence deletion requires policy, age/retention checks, and explicit operator authority.
- Completed/reviewed work follows project integration policy. Commit, push, PR, merge, deployment, and cleanup are distinct effects.

## 15. Status, explanations, and UI parity

Every important decision must be explainable through the CLI and the authenticated local UI:

- why a project is or is not eligible today;
- why a route was selected or rejected;
- which quota evidence and forecast were used;
- which setting/policy revision supplied an effective value;
- why an action was automatic, denied, or sent for approval;
- where a run is executing and what its sandbox attestation proves;
- whether credentials were available without revealing them;
- why a handoff, pause, retry, or cleanup occurred;
- what the next safe action and wake time are.

The CLI and web interface call the same control-plane operations. The UI must not invent separate policy, settings, approval, or lifecycle semantics.

## 16. Current implementation disposition

| Area | Current disposition relative to this Bible |
| --- | --- |
| SQLite tasks, dependencies, events, leases, claims, recovery | Reusable internal control-plane foundation; must move behind project-oriented operations. |
| Scheduler and route evidence | Reusable, but normal exact pins and one-task daemon acknowledgements must not define routine UX. |
| Quota surfaces, observations, overrides, forecasts, reservations | Core reusable capability; connect to continuous multi-project routing and checkpoint handoff. |
| Calendar | Partial and semantically incorrect for `WWWOOBB`; migrate from `W`/`O` day classes and task affinity to `W`/`O`/`B` system days plus project affinity. |
| Policy and approvals | Useful effect classes and single-use action-digest primitives exist; real agent approval brokers and project-level configuration are missing. |
| Notifications | Durable local rows and acknowledgement exist for a few events; transactional outbox and actual delivery adapters are missing. |
| Secrets | Environment, protected-file, and macOS Keychain references exist; secret-management commands, Linux keyring, rotation, and subscription credential projection are missing. |
| Container backends | Conformance foundations exist; real subscription-agent execution through attested containers is missing. |
| Codex subscription adapter | Keep as a narrow host-side read-only patch fallback and compatibility diagnostic; it is not the normal target execution path. |
| Claude Code, Antigravity, AoE/ACP | Required product capabilities remain unimplemented. |
| Direct OpenAI/Anthropic API | Preserve the bounded transport/accounting work; integrate only as explicitly enabled project routes. |
| Worktree cleanup | Missing normal lifecycle operation; add automatic and operator-previewed reconciliation. |
| Settings/config explain | Specified in design but not implemented as a coherent system. |
| Read-only dashboard | Reusable presentation foundation; add mutations only through the same authenticated, CSRF-protected control operations. |
| Packaging | Current native archives remain useful for compatibility testing; a future project-supervisor release requires a new acceptance boundary. |

## 17. Definition of the project-supervisor MVP

The intended MVP is not complete until, on each supported platform, machine evidence proves that an operator can:

1. initialise Garnish and configure secure native state;
2. configure the `WWWOOBB` system calendar and register at least one work project and one non-work project;
3. register and probe at least Codex and Claude subscription profiles;
4. start multiple projects without manually creating internal tasks;
5. observe Garnish schedule only projects eligible for the current `W`, `O`, or `B` day;
6. run low-risk project work autonomously inside an attested secure container;
7. receive one actionable notification when an agent requests a valid but elevated action;
8. approve or deny that exact action and observe safe resume or bounded handoff;
9. observe low Codex capacity produce a checkpoint and automatic Claude handoff without paid fallback;
10. inspect status that explains calendar, route, quota, settings, approval, container, and next-wake decisions;
11. observe failure clean up container/worktree/branch state without losing bounded evidence;
12. use protected credentials without any secret value appearing in arguments, database, logs, notifications, patches, or artifacts;
13. stop or emergency-stop all work with durable termination and cleanup evidence.

All normal acceptance uses fake agents, fake quotas, fake approval requests, and fake secret values. Real Codex/Claude subscription and API tests are separate, labelled, minimal, non-retrying, and explicitly opted into by the operator.

## 18. Phased to-be delivery

The `TB` phases below are product-realignment phases and do not renumber or rewrite historical Phase 1–4 evidence.

### TB-0 — Contract and migration baseline

Deliver:

- this Bible reviewed as the intended operator contract;
- command grammar and stable JSON schemas for every normal family;
- project, calendar, settings, notification, approval, secret, sandbox, cleanup, and handoff state-machine specifications;
- an as-is/to-be gap matrix for every existing service and command;
- schema migration plan for `W`/`O`/`B` days and project affinity;
- ADR 0016 establishing that the narrow host patch adapter is a fallback, not the primary execution path;
- quota-free acceptance fixtures written before implementation.

Exit evidence: documentation consistency checks and executable CLI/parser fixtures for the agreed command surface. No provider or container action is required.

The frozen TB-0 artifacts are the [CLI/JSON contract](tb0-command-contract.md), [state-machine contracts](tb0-state-contracts.md), [current gap matrix](tb0-gap-matrix.md), [schema migration plan](tb0-schema-migration.md), [acceptance plan](tb0-acceptance.md), machine-readable [`garnish.cli/v1alpha1` contract](contracts/tb0-cli-v1.json), and [ADR 0017](decisions/0017-tb0-cli-contract.md).

TB-0 machine evidence and its exact non-implementation boundary are recorded in the [TB-0 exit report](tb0-exit-report.md).

### TB-1 — Project-oriented supervisor with fake execution

Implementation status: exited on 2026-07-21 with the quota-free evidence recorded in the [TB-1 exit report](tb1-exit-report.md). The fixture-only and deferred boundaries in that report remain controlling; this status does not imply completion of TB-2 or real-agent execution.

Deliver:

- system configuration and `config explain`;
- corrected `WWWOOBB` calendar and project affinity;
- project start/pause/resume/stop/status lifecycle;
- high-level objectives/backlog source and internal task generation;
- continuous multi-project scheduler behind the service interface;
- automatic routing and fake quota-triggered handoff;
- automatic terminal cleanup and maintenance preview;
- current task-oriented commands moved behind the advanced compatibility surface.

Exit evidence: a quota-free multi-project scenario where work and non-work projects run only on eligible days, low fake Codex quota hands work to fake Claude, and no task ID is required from the operator.

### TB-2 — Settings, approval, notification, and secret foundation

Deliver:

- complete layered settings/policy revision and explanation path;
- structured approval broker with exact action digest, expiry, consume, deny, resume/restart, and audit;
- transactional notification outbox, local inbox, desktop delivery where supported, and UI/CLI acknowledgement;
- secret-management commands, macOS Keychain, Linux keyring where available, protected-file fallback, rotation metadata, and redaction tests;
- fake agent permission requests and fake credential projection.

Exit evidence: contained actions auto-allow, elevated actions produce exactly one approval/notification, decisions are replay-safe, and secret canaries are absent from every durable/output surface.

### TB-3 — Real attested-container Codex subscription execution

Deliver:

- hardened Docker/rootless-Podman/Apple-Container project profiles;
- task-scoped Codex subscription-auth projection or an explicitly documented alternative boundary;
- exact model/profile selection where supported;
- structured Codex execution, checkpoints, cancellation, handoff, and cleanup;
- autonomous project-permitted writes/builds/tests inside the container;
- host patch adapter retained only as fallback/diagnostic.

Exit evidence: quota-free fake conformance on every platform plus one separately acknowledged minimal live Codex project run per supported runtime boundary. No automatic retry of uncertain subscription use.

### TB-4 — Claude Code and subscription switching

Deliver:

- Claude Code profile/probe, task-scoped auth projection, stream parsing, checkpoint/resume, model selection, and container execution;
- allowed/disallowed tool policy and Garnish permission-prompt broker;
- Codex-to-Claude and Claude-to-Codex structured handoff;
- quota-driven route switching across multiple active projects;
- continuity and verification evidence across adapter changes.

Exit evidence: fake end-to-end switching plus separately opted-in minimal live Claude and cross-agent checkpoint tests. Routine contained commands must not require operator approval.

### TB-5 — Explicit API fallback and additional agents

Deliver:

- project-level policy stating whether low subscription quota may consider paid API routes;
- reusable OpenAI/Anthropic budget, pricing, reservation, attempt, settlement, and patch/tool boundaries behind project routing;
- user-facing budget configuration and approval notifications without manual per-task request plans;
- Antigravity and later adapters admitted only through versioned capability, sandbox, approval, quota, and handoff contracts;
- controlled MCP/skills execution only after its own lifecycle, secret, network, approval, and context-limit evidence.

Exit evidence: fake subscription exhaustion selects another subscription first and paid API only when explicitly enabled and fully budgeted; live paid tests remain one-request opt-ins.

### TB-6 — Complete operator surfaces and release readiness

Deliver:

- authenticated local web parity for project control, settings, approvals, notifications, quotas, status, and evidence;
- optional remote notifications/approvals over a separately secured design;
- service installation/upgrade/rollback, backup/restore, retention, and support-bundle workflows;
- signed/notarized or otherwise platform-appropriate distribution decisions;
- updated installation and use documentation centred on project supervision rather than internal tasks.

Exit evidence: clean-install packaged acceptance on every claimed platform, multi-project soak/recovery tests, accessibility/security review of the UI, and no reliance on development-only commands.

## 19. Decisions resolved for the first project-supervisor MVP

TB-0 resolves the earlier open questions as follows:

- The first backlog is Garnish-local user objectives; TB-1 creates one deterministic implementation task per objective. Agent planning/import remain later boundaries.
- The exact normal/advanced names, operands, JSON schemas, and exit codes are frozen in `garnish.cli/v1alpha1`.
- Subscription auth must be a Garnish-owned provider-specific broker/profile isolated from coding shells. Host user auth is not copied into containers; unsupported platforms/routes remain ineligible or use an honestly labelled fallback.
- Codex first targets a version-pinned structured app-server adapter; Claude targets version-pinned stream JSON plus permission-prompt integration. Prompt scraping is forbidden.
- Secret backend order is macOS Keychain, usable Linux Secret Service, then explicit mode-`0600` protected file for headless Linux/WSL2.
- Cleanup and evidence retention defaults are fixed in the TB-0 command contract, with quarantine rather than deletion for unexpected state.
- Initial notifications use the durable local inbox and best-effort native desktop delivery. Quiet hours delay non-critical delivery only.
- Working-hour windows are excluded from the first project-supervisor MVP; day classes and safe-boundary checkpoints are sufficient.
- Integration defaults to review plus explicit `project apply`; no commit, branch switch, push, PR, merge, or deployment is implied.
- Garnish owns container construction, attestation, authority, and cleanup. AoE may supervise lifecycle only behind a versioned adapter.

If provider-specific subscription authentication cannot satisfy the isolated broker/profile requirement, that adapter remains blocked for the secure-container phase rather than weakening the boundary silently.
