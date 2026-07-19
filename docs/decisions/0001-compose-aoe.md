# ADR 0001: Compose with Agent of Empires

- Status: accepted
- Date: 2026-07-19

## Context

Harness Garnish needs PTY/session supervision, worktrees, containers, status, cancellation, and optional remote visibility. Reimplementing all of these before validating the scheduler would delay the safe vertical slice.

AoE is actively maintained and MIT-licensed. It exposes an authenticated HTTP API, CLI, ACP integration, and versioned plugin API, and supports the initial agents and container runtimes.

## Decision

Use AoE as the first execution-plane implementation behind Garnish's `ExecutionPlane` interface. Prefer authenticated loopback HTTP and ACP; use CLI calls for probe/setup functions. A Garnish AoE plugin is optional and must request minimal capabilities.

Garnish remains authoritative for projects, tasks, leases, policy, quotas, approvals, handoffs, run evidence, and verification. AoE identifiers are external references, never primary keys. A fake plane and reduced internal process plane prevent lock-in.

Do not accept AoE's default container configuration as proof of security. Garnish must inspect or create the effective sandbox and attest its actual mounts and grants.

## Consequences

- Phase 1 reaches real session supervision sooner.
- AoE version drift requires fixtures and a supported-version matrix.
- Some functionality may require a hardened AoE profile or Garnish-owned sandbox backend.
- Swapping execution planes does not migrate proprietary conversation state; it creates a structured handoff from repository state and evidence.

