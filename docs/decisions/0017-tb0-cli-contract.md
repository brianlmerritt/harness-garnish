# ADR 0017: Freeze the TB-0 project-supervisor CLI contract before implementation

- Status: accepted
- Date: 2026-07-21

## Context

ADR 0016 changes Harness Garnish from a task-operated compatibility CLI into a project supervisor. Implementing commands piecemeal without first fixing their grammar, output, state, migration, safety, and compatibility boundaries would risk another workflow in which the user manually coordinates internal control-plane records.

The current implementation and its evidence must remain operable while the new interface is built. The pivot also needs explicit decisions for objectives/backlog, result integration, calendars, subscription authentication, agent approvals, secret stores, notification delivery, cleanup retention, and AoE authority.

## Decision

Freeze `garnish.cli/v1alpha1` as the TB-0 contract in [`../tb0-command-contract.md`](../tb0-command-contract.md) and its machine-readable companion [`../contracts/tb0-cli-v1.json`](../contracts/tb0-cli-v1.json). Runtime implementation begins only after quota-free contract fixtures pass.

The normal interface has 18 project-supervisor families: `init`, `doctor`, `status`, `service`, `config`, `calendar`, `project`, `objective`, `agent`, `quota`, `route`, `approval`, `notification`, `secret`, `policy`, `events`, `ops`, and `maintenance`. Existing task, scheduler, runtime, API-plan, MCP, and diagnostic operations move beneath an `advanced` compatibility gateway until replacement evidence and deprecation requirements are satisfied.

The first backlog source is user-authored Garnish-local objectives. TB-1 deterministically creates one internal implementation task per objective. Agent planning and external issue import remain deferred trust boundaries.

The default integration policy is review. A verified result is represented by durable patch/base/scope/verification evidence, and Garnish cleans its execution worktree. `project apply` may apply the unchanged result to a clean matching registered checkout, but it does not commit, switch branches, push, merge, open a PR, or deploy.

Calendars use system day classes `W`, `O`, and `B` with project affinity `work`, `non_work`, or `both`. Optional working-hour windows are excluded from the first project-supervisor MVP.

Autonomous agent execution requires structured lifecycle and approval events plus an independently attested container. Codex targets a version-pinned structured app-server adapter; Claude targets version-pinned stream JSON and permission-prompt integration. Terminal prompt scraping and automated keystroke confirmation are forbidden. Subscription authentication must use a Garnish-owned provider-specific broker/profile isolated from coding shells; host user authentication is not silently copied into containers. Where these requirements are not met, the route is ineligible and ADR 0014 remains a separately labelled Codex fallback.

The initial secret backend order is macOS Keychain, usable Linux Secret Service, then an explicitly selected mode-`0600` protected file for headless Linux/WSL2. The initial notification boundary is durable local inbox plus best-effort native desktop delivery; quiet hours delay only non-critical delivery, not event or approval creation.

Garnish owns sandbox construction, attestation, and cleanup authority. AoE may supervise sessions behind a versioned adapter but its defaults cannot establish policy or secure-container status.

The default cleanup and retention periods are fixed in the command contract. Unexpected/unowned Git or filesystem state is quarantined and reported rather than deleted.

## Consequences

- The machine contract can be tested before any target command mutates runtime state.
- Current packaged behaviour remains accessible while normal documentation and future implementation move to project-first commands.
- Human-readable output may evolve, but JSON envelopes, resource-required fields, and exit meanings require versioned compatibility.
- Schema 20 must migrate through the staged, inert-by-default plan in [`../tb0-schema-migration.md`](../tb0-schema-migration.md).
- The subscription-auth isolation decision may require provider-specific engineering or constrain eligible platforms; Garnish must report that honestly rather than mount a writable host credential directory.
- Future changes to these decisions require a superseding ADR and updated contract fixtures.

