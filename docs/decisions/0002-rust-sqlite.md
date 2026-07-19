# ADR 0002: Rust control plane and SQLite state

- Status: accepted
- Date: 2026-07-19

## Context

The control plane is a long-running local supervisor with concurrent state transitions, process cancellation, bounded resource use, stable schemas, and cross-platform distribution requirements. Markdown cannot provide atomic transitions or leases.

## Decision

Implement `garnish` in stable Rust and store canonical state in versioned SQLite using WAL mode, foreign keys, busy timeouts, and explicit transactions. Back up the database before every migration. Generate bounded human-readable project projections from committed state.

Use ULID-style stable identifiers or an equivalently sortable, collision-resistant identifier. Persist all timestamps as UTC plus the original local timezone where user intent depends on it.

## Consequences

- A single binary is practical on macOS, Linux, and WSL2.
- Rust increases initial implementation cost but reduces process, concurrency, and schema-boundary ambiguity.
- SQLite remains single-host canonical state. Multi-host operation uses explicit worker protocols, not shared-filesystem database access.

