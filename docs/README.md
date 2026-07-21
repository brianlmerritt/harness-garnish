# Design documentation

These documents contain the design baseline and phased implementation evidence. Design documents describe contracts; exit reports distinguish implemented and tested behavior from deferred work.

| Document | Purpose |
| --- | --- |
| [Phase 0 discovery](phase-0-discovery.md) | Environment evidence, adopt/compose decision, scope, and open implementation risks |
| [Architecture](architecture.md) | Boundaries, components, lifecycle, portability, state, and recovery |
| [Threat model](threat-model.md) | Assets, trust boundaries, threats, controls, and residual risks |
| [Data model](data-model.md) | Canonical SQLite entities, invariants, transitions, events, and projections |
| [Policy model](policy-model.md) | Configuration precedence, risk classes, Git rules, quotas, API budgets, and approvals |
| [Adapter contracts](adapter-contracts.md) | Execution-plane, agent, sandbox, quota, API, notification, and updater interfaces |
| [MVP acceptance](mvp-acceptance.md) | Measurable vertical-slice and Phase 1 exit criteria |
| [Phase 1 exit report](phase-1-exit-report.md) | Implemented scope, test evidence, waivers, and residual risks |
| [Phase 2 plan](phase-2-plan.md) | Durable scheduler, day-aware calendars, recovery, and cross-platform acceptance |
| [Phase 2 Linux midpoint](phase-2-linux-midpoint.md) | Linux scheduler, process, permission, and rootless-Podman capability evidence |
| [Phase 2 Linux container conformance](phase-2-linux-container-conformance.md) | Real rootless-Podman and Docker sandbox attestation/lifecycle evidence |
| [Phase 2 WSL2 exit](phase-2-wsl2-exit.md) | WSL2 path-policy, lifecycle, backup, permissions, and runtime-selection evidence |
| [Phase 2 exit report](phase-2-exit-report.md) | Final schema-7 macOS, Linux, and WSL2 acceptance evidence and residual scope |
| [Phase 3 plan](phase-3-plan.md) | Multi-agent capability evidence, quota providers/reservations, deterministic routing, and independent verification |
| [Phase 3 exit report](phase-3-exit-report.md) | Final schema-14 macOS, Linux, and WSL2 routing, forecasting, UI, and verifier acceptance evidence |
| [Phase 4 plan](phase-4-plan.md) | Budgeted OpenAI/Anthropic API agents, controlled remote approvals, and MCP/skill/ACP boundaries |
| [Phase 4 portability checkpoint](phase-4-portability-checkpoint.md) | Schema-15 API accounting and provider-fixture evidence from macOS, Linux, and WSL2 |
| [Phase 4 live API checkpoint](phase-4-live-api-checkpoint.md) | Operator-reported one-request response and isolated-patch smoke results for OpenAI and Anthropic |
| [Phase 4 live Codex checkpoint](phase-4-live-codex-checkpoint.md) | Codex 0.144.6 compatibility regression and passing one-task subscription patch evidence |
| [CLI Bible](cli-bible.md) | Authoritative to-be project-supervisor product, command, settings, calendar, routing, approval, notification, secret, sandbox, lifecycle, and phased-delivery contract |
| [Archived narrow CLI MVP boundary](archive/cli-mvp.md) | Historical implemented Codex subscription/API task-oriented compatibility boundary and evidence context; not the intended product interface |
| [CLI MVP exit report](cli-mvp-exit-report.md) | Final source-run CLI acceptance, platform/live evidence, security boundary, and deferred release work |
| [TB-0 CLI and JSON contract](tb0-command-contract.md) | Exact normal/advanced grammar, stable JSON/exit semantics, project review/apply, agent boundaries, and retention defaults |
| [TB-0 state-machine contracts](tb0-state-contracts.md) | Project, objective, run, calendar, settings, approval, notification, secret, sandbox, cleanup, handoff, and review transitions |
| [TB-0 gap matrix](tb0-gap-matrix.md) | Disposition of every current command family and source service relative to the project-supervisor design |
| [TB-0 schema migration](tb0-schema-migration.md) | Staged schema-20 migration, inert activation, data mapping, rollback, and fixture requirements |
| [TB-0 acceptance](tb0-acceptance.md) | Quota-free machine evidence required before TB-1 runtime work begins |
| [TB-0 exit report](tb0-exit-report.md) | Passing contract/parser, formatting, strict Clippy, full quota-free regression, and precise residual implementation boundary |
| [Installation and packaging](installation.md) | Source installation, native archives, checksums, state, upgrades, uninstall, and release limitations |
| [CLI operator guide](operator-guide.md) | Task-oriented fixture, Codex subscription, paid API, dashboard, backup, and recovery workflows |
| [Distributable CLI MVP exit report](cli-package-exit-report.md) | Clean-tree macOS, native-Linux, and WSL2 native archive acceptance and residual release scope |
| [ADRs](decisions/README.md) | Decisions that constrain implementation |

Every material change to these boundaries requires an ADR amendment or a superseding ADR. Documentation may describe a future capability only when it is clearly labelled as planned.
