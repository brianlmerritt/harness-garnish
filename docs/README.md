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
| [CLI MVP boundary](cli-mvp.md) | Included Codex subscription/API workflows, review handoff, acceptance evidence, and explicit deferrals |
| [CLI MVP exit report](cli-mvp-exit-report.md) | Final source-run CLI acceptance, platform/live evidence, security boundary, and deferred release work |
| [ADRs](decisions/README.md) | Decisions that constrain implementation |

Every material change to these boundaries requires an ADR amendment or a superseding ADR. Documentation may describe a future capability only when it is clearly labelled as planned.
