# ADR 0006: API providers and self-update

- Status: accepted
- Date: 2026-07-19

## Decision

Provide distinct OpenAI and Anthropic API-provider adapters. They are disabled per project unless explicitly enabled with hard monetary and/or token budgets, allowed models, tools, network, secrets, and retry ceilings. Subscription CLI usage and API billing are separate resource pools.

Support manual and automatic Garnish updates. Both verify signed release metadata, hashes, channel, compatibility, migration plan, and rollback availability. Automatic mode may download to a staging location but activates only at an idle/checkpoint boundary. Database backup and binary rollback are mandatory; agents cannot change update policy during their run.

## Consequences

- API use cannot silently consume paid budget when subscriptions are exhausted.
- Current model aliases stay configuration data and can change without schema migrations.
- The updater is a privileged integration with stricter policy than normal task execution.

