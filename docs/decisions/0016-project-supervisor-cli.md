# ADR 0016: Project-supervisor CLI and project-first operating model

- Status: accepted
- Date: 2026-07-21

## Context

The narrow CLI MVP proved durable task state, quota-aware routing, isolated worktrees, Codex subscription patch execution, explicitly budgeted API execution, detached verification, review evidence, and native packaging. Its normal workflow nevertheless requires an operator to create, pin, prepare, route, execute, review, and clean up individual internal tasks.

Harness Garnish exists to supervise multiple projects while balancing coding-agent subscription capacity, schedules, policies, and optional paid fallbacks. Making internal task mechanics the primary interface transfers orchestration work back to the user and does not provide that product.

The intended system also needs explicit answers for the user's `WWWOOBB` calendar, project affinity, Codex and Claude subscription routing, quota-triggered checkpoint handoff, agent approval requests, notifications, layered settings, protected credentials, sandbox authority, and failed-run cleanup. These concerns must be designed as one operator system rather than added as unrelated task commands.

## Decision

Projects are the primary operator unit. The normal CLI starts, pauses, resumes, stops, inspects, and configures project supervision. Garnish owns the internal decomposition, task claims, route decisions, run lifecycle, checkpoints, handoffs, verification, and cleanup. Existing task-oriented commands remain an advanced compatibility and diagnostic surface; they do not define the target user experience.

The authoritative detailed product and command contract is [`../cli-bible.md`](../cli-bible.md). Its phased `TB` acceptance boundaries govern implementation claims for the project-supervisor experience. Historical Phase 1–4 and narrow CLI MVP evidence remains historical fact and is not relabelled as evidence for the new boundary.

The target calendar has system day classes `W`, `O`, and `B`, with a configurable weekly pattern such as `WWWOOBB`. Projects declare `work`, `non-work`, or `both` affinity and internal tasks inherit that eligibility.

Codex and Claude subscription profiles are first-class preferred routes. Garnish checkpoints and hands work between eligible subscription routes as capacity and policy require. Paid API routes remain separately enabled, budgeted, credentialed, and policy-gated; low subscription capacity alone never authorises paid use.

Routine permitted development commands execute autonomously only inside an independently attested secure container. Garnish, rather than the agent, brokers structured elevated-action requests through exact, expiring, single-use approvals and durable notifications. Settings are layered and explainable, secrets remain in protected stores and receive task-scoped projection, and Garnish automatically reconciles its owned execution environments under explicit evidence-retention policy.

ADR 0014 remains the accepted security and execution decision for the implemented one-task host-side Codex patch adapter. This ADR supersedes only its implied role as the target CLI MVP/product interface. That adapter becomes a compatibility fallback and diagnostic path until a later ADR and machine evidence establish real subscription-agent execution through the attested-container boundary.

## Consequences

- New normal workflows must be designed and tested around projects and status, not sequences of task IDs and daemon acknowledgements.
- The current packaged CLI remains usable as a compatibility build while the `TB` phases are implemented; this ADR does not claim the project-supervisor MVP exists today.
- The scheduler, quota, task, lease, worktree, evidence, verifier, API-accounting, and policy foundations remain reusable behind the new interface.
- Calendar storage and evaluation require a deliberate migration from current `W`/`O` system days plus task-level affinity.
- Codex and Claude container execution, structured approval integration, notification delivery, settings explanation, credential projection, and automatic cleanup require new acceptance evidence.
- Direct paid API work remains useful but is exposed through project policy rather than routine per-task request-plan construction.
- Documentation and status output must distinguish implemented compatibility behaviour from the to-be product contract.
