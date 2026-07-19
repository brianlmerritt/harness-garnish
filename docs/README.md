# Design documentation

These documents constitute the Phase 0 design baseline. They describe intended contracts and acceptance criteria, not implemented behaviour.

| Document | Purpose |
| --- | --- |
| [Phase 0 discovery](phase-0-discovery.md) | Environment evidence, adopt/compose decision, scope, and open implementation risks |
| [Architecture](architecture.md) | Boundaries, components, lifecycle, portability, state, and recovery |
| [Threat model](threat-model.md) | Assets, trust boundaries, threats, controls, and residual risks |
| [Data model](data-model.md) | Canonical SQLite entities, invariants, transitions, events, and projections |
| [Policy model](policy-model.md) | Configuration precedence, risk classes, Git rules, quotas, API budgets, and approvals |
| [Adapter contracts](adapter-contracts.md) | Execution-plane, agent, sandbox, quota, API, notification, and updater interfaces |
| [MVP acceptance](mvp-acceptance.md) | Measurable vertical-slice and Phase 1 exit criteria |
| [ADRs](decisions/README.md) | Decisions that constrain implementation |

Every material change to these boundaries requires an ADR amendment or a superseding ADR. Documentation may describe a future capability only when it is clearly labelled as planned.

