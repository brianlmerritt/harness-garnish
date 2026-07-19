# Harness Garnish

Harness Garnish is a local control plane for quota-aware, policy-controlled AI-assisted software development. It coordinates existing coding-agent CLIs and optional API-backed agents, while keeping scheduling, approvals, canonical state, verification, and recovery outside the agents themselves.

The project is in architecture and discovery. Phase 0 is documented under [`docs/`](docs/README.md); no executable has been scaffolded yet.

## Confirmed direction

- `garnish` will be a small Rust control plane with SQLite as canonical state.
- Agent of Empires (AoE) is the first execution-plane integration, behind a replaceable adapter.
- Docker, rootless Podman, and Apple Container are independent sandbox backends selected by capability probes and policy.
- Codex CLI, Claude Code, and Antigravity CLI are the first agent adapters.
- OpenAI and Anthropic APIs are opt-in per project, with hard project budgets.
- Subscription limits are represented as multiple independently configurable quota surfaces, never as one invented total.
- The default checkpoint interval is five minutes and may be shortened by the quota guard.
- Git mutation policy is project-specific. This repository's branches and commits remain user-managed.

Harness Garnish is not an API proxy, a provider-limit bypass, an autonomous merge/deploy service, or a claim that proprietary conversation state is portable between agents.

## Phase 0 documents

- [Discovery and decisions](docs/phase-0-discovery.md)
- [Architecture](docs/architecture.md)
- [Threat model](docs/threat-model.md)
- [Data model](docs/data-model.md)
- [Policy model](docs/policy-model.md)
- [Adapter contracts](docs/adapter-contracts.md)
- [MVP acceptance tests](docs/mvp-acceptance.md)
- [Architecture decision records](docs/decisions/README.md)

## Repository authority

The user manages branches and commits for this repository. Agents may edit the current checkout when explicitly asked, but must not create or switch branches, commit, push, open pull requests, merge, or alter remotes.

