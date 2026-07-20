# Design documentation

These documents contain the Phase 0 design baseline and the Phase 1 implementation evidence. Design documents describe contracts; the exit report distinguishes implemented and tested behavior from deferred work.

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
| [Phase 2 WSL2 exit](phase-2-wsl2-exit.md) | WSL2 path-policy, lifecycle, backup, permissions, and runtime-selection evidence |
| [ADRs](decisions/README.md) | Decisions that constrain implementation |

Every material change to these boundaries requires an ADR amendment or a superseding ADR. Documentation may describe a future capability only when it is clearly labelled as planned.
