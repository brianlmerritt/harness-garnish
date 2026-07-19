# Phase 0 discovery

Status: complete on 2026-07-19. This is an architecture baseline; implementation has not begun.

## Product boundary

Harness Garnish owns deterministic project/task state, task scheduling, quota guards, routing, approvals, policy evaluation, run evidence, recovery, and verification. Existing tools may own terminal sessions, process supervision, worktrees, or containers only through explicit, versioned adapters.

The first implementation must remain useful when AoE is unavailable. Fake adapters provide deterministic tests; a reduced built-in process adapter provides a recovery path; AoE is the preferred full execution plane.

## Confirmed user decisions

- Platforms: macOS, Linux, and Windows through WSL2.
- Sandboxes: Docker, rootless Podman, and Apple Container, chosen per host and task.
- Execution plane: Agent of Empires first, with the ability to replace it or build an internal plane later.
- Agents in initial scope: Codex CLI, Claude Code, and Antigravity CLI. Phase 1 includes real adapters for all three plus quota-free fixtures and opt-in smoke tests.
- Future candidates: OpenClaw, Pi, Hermes Agent, and additional capability-declared adapters.
- Autonomy: low-risk work may proceed autonomously only in an independently verified secure container. Git permissions remain project-specific and may be stricter or broader than defaults.
- Direct APIs: OpenAI and Anthropic abstractions are in scope, disabled by default per project, with explicit monetary/token budgets when enabled.
- Quotas: model each subscription window or paid-overage balance separately. Defaults use a 20% reserve and five-minute checkpoints.
- State: native/XDG locations, SQLite WAL, pre-migration backup, project Markdown projections, and explicit encrypted export/import.
- Updates: signed update metadata with user-selectable manual or automatic policy; automatic activation must be rollback-safe.
- Notifications: local first; Tailscale or SSH remote approval later.
- Distribution: public Apache-2.0 project; do not derive code from incompatible source-available projects.
- Repository Git: the user manages branches and commits.

## Local evidence

Read-only probes on 2026-07-19 found:

| Component | Evidence | Phase 0 conclusion |
| --- | --- | --- |
| Host | macOS 26.5.2, Darwin arm64 | Primary development host; Linux and WSL2 require CI/worker fixtures. |
| Codex CLI | `codex-cli 0.144.5`; `codex exec` supports JSONL, output schema, resume, review, sandbox selection, timeout through the supervisor, and explicit unsafe bypass flags | Prefer structured non-interactive mode. Never enable the bypass flag unless an external sandbox passes policy. |
| Claude Code | `2.1.215`; print mode, JSON/stream-JSON, JSON schema, resume/fork, allowed/disallowed tools, permission modes, hooks, and budget flag are exposed | Prefer stream-JSON. In a verified external sandbox use a scoped non-interactive policy; retain external command filtering and budgets. |
| Antigravity CLI | `agy 1.1.4`; print mode, five-minute default print timeout, resume, model/agent selection, `accept-edits`/`plan`, and sandbox flags are exposed | Treat output as text until a stable structured schema is discovered. Supervise via process or PTY and preserve raw logs. |
| Docker | CLI 28.0.1; `desktop-linux` context configured; daemon unavailable during the probe | Phase 1 must report an unhealthy backend rather than assuming Docker is usable. |
| Podman / Apple Container | Not found on the probed `PATH` | Implement capability probes and CI fixtures before claiming support. |
| AoE | Not found on the probed `PATH` | Phase 1 installation is user-approved setup; adapters must also test supported version ranges and API schema. |

The user's installed agent binaries are in `~/.local/bin`, which was not present in the process `PATH`. `garnish doctor` must inspect configured paths and common user-bin locations without silently changing shell profiles.

## Adopt, compose, or build

### Selected: compose with Agent of Empires

[Agent of Empires](https://github.com/agent-of-empires/agent-of-empires) is MIT-licensed and shipped v1.13.0 on 2026-07-16. It provides tmux lifecycle management, worktrees, multi-repository sessions, Docker/Podman/Apple Container support, ACP, an authenticated HTTP API, and a capability-gated plugin API. The documented HTTP API is intentionally small: create sessions, send input, poll status, and capture terminal output. Its plugin manifest has an explicit API version and AoE semver constraint.

AoE is composed, not embedded or forked. Garnish will pin supported versions and own task state, leases, quotas, policy, approvals, and verification. The AoE adapter may use:

1. authenticated loopback HTTP for session lifecycle;
2. ACP where it gives typed events and approvals;
3. the CLI for health checks or functions absent from the HTTP API;
4. a narrowly capability-gated plugin only when the public interfaces are insufficient.

AoE's stock sandbox documentation describes automatic credential/config sharing and broad project mounts. That does not satisfy Garnish's secure-container definition by itself. Garnish must generate a hardened profile or bypass AoE's sandbox creation and provide the sandbox through its own backend. Until tests prove the resulting mounts, environment, network, user, and resources, policy must not label an AoE session `secure_container=true`.

### Alternatives retained

- [Agent Deck](https://github.com/asheshgoplani/agent-deck), MIT and actively maintained, remains the first alternate terminal execution plane.
- [DeerFlow](https://github.com/bytedance/deer-flow), MIT, may later be an optional planning/API-agent provider, not the control plane.
- [Dagger Container Use](https://github.com/dagger/container-use), Apache-2.0, may be an optional sandbox adapter after security/lifecycle tests.
- [CodexBar](https://github.com/steipete/CodexBar), MIT, is the preferred macOS quota snapshot source through machine-readable output. Current public CLI documentation confirms `usage`, `cost`, and loopback `serve`; Garnish implements the policy guard itself.
- [Tokscale](https://github.com/junhoyeo/tokscale), MIT, supplies historical and secondary quota telemetry. Local token counts are never relabelled as authoritative remaining subscription quota.
- [Ivy Tendril](https://github.com/Ivy-Interactive/Ivy-Tendril) uses FSL-1.1-ALv2. No code or protected implementation is copied into this public competing product.

## Vendor interface evidence

- [AoE HTTP API](https://www.agent-of-empires.com/docs/api/) documents bearer authentication, read-only operation, session creation, literal input delivery, status polling, and bounded terminal capture.
- [AoE plugin API](https://www.agent-of-empires.com/docs/plugin-api/) documents strict manifests, API/host version constraints, capability approval, and argv-array process definitions.
- [AoE sandbox guide](https://www.agent-of-empires.com/guides/sandbox/) documents Docker and reveals credential/mount defaults that Garnish must harden.
- [AoE Podman guide](https://www.agent-of-empires.com/guides/podman/) documents rootless-friendly use and SELinux implications.
- [Anthropic CLI reference](https://docs.anthropic.com/en/docs/claude-code/cli-usage) documents print mode, structured streams, permission modes, tool controls, resume, and programmatic output.
- [Google Antigravity CLI codelab](https://codelabs.developers.google.com/sdd-agy-cli) documents `agy --print`, sandboxing, resume, timeout, skills, and MCP.
- [OpenAI model documentation](https://developers.openai.com/api/docs/models) establishes Responses API availability and capability metadata for current API models. Model aliases remain configuration, not baked into Garnish's schema.

The official OpenAI documentation MCP was added to the local Codex configuration during discovery. Its capabilities will become available after the client refreshes; Phase 0 therefore uses the locally installed Codex help output for exact CLI 0.144.5 flags and official OpenAI web documentation only for API-level claims.

## Phase 1 prerequisites and risks

- Install or explicitly configure a supported AoE binary; do not auto-install it as a side effect of `garnish doctor`.
- Start Docker Desktop for the first real backend smoke test, or select another healthy backend.
- Establish opt-in, task-scoped subscription credential projection. Do not mount writable global credential directories.
- Determine whether AoE can suppress every stock automatic mount. If not, use Garnish-owned container creation for the vertical slice and use AoE only for session supervision.
- Add Linux and WSL2 CI runners or reproducible VM fixtures before declaring those platforms supported.
- Treat Antigravity output and flags as version-specific because its current CLI lacks a documented structured event stream.

## Phase 0 exit evidence

Phase 0 is complete when all documents in `docs/README.md` exist, ADRs are accepted, every requirement in the build brief maps to a documented component or deferred phase, and the Phase 1 vertical slice has binary pass/fail criteria. No application scaffold or provider-quota-consuming test is part of Phase 0.
