# Architecture decision records

| ADR | Decision | Status |
| --- | --- | --- |
| [0001](0001-compose-aoe.md) | Compose with Agent of Empires behind a replaceable execution-plane adapter | Accepted |
| [0002](0002-rust-sqlite.md) | Rust control plane with SQLite canonical state | Accepted |
| [0003](0003-portable-sandbox-backends.md) | Separate Docker, rootless Podman, and Apple Container backends | Accepted |
| [0004](0004-quota-surfaces-and-checkpoints.md) | Multi-surface quota policy and five-minute checkpoints | Accepted |
| [0005](0005-layered-project-policy.md) | Layered, project-specific policy with secure-container autonomy | Accepted |
| [0006](0006-api-providers-and-updates.md) | Opt-in budgeted APIs and rollback-safe self-update | Accepted |
| [0007](0007-local-operator-interface.md) | Authenticated loopback operator interface with CLI parity | Accepted |
| [0008](0008-historical-usage-forecasting.md) | Explicit usage evidence with conservative identity-scoped forecasts | Accepted |
| [0009](0009-independent-verifier-runs.md) | Separate verifier selection, run identity, clean worktree, and evidence | Accepted |
| [0010](0010-direct-api-transport.md) | Direct TLS provider transport with explicit paid-smoke opt-in and no hidden replay | Accepted; scheduler activation boundary superseded by 0011 |
| [0011](0011-explicit-api-scheduler-execution.md) | Explicitly pinned paid API scheduler execution with a session acknowledgement | Accepted; response-only limitation partially superseded by 0012 |
| [0012](0012-controlled-api-patch-submission.md) | Controlled, exactly scoped API patch submission to an isolated worktree | Accepted |
| [0013](0013-default-deny-mcp-registration.md) | Default-deny MCP registration before any execution boundary | Accepted |
| [0014](0014-controlled-codex-subscription-patch-execution.md) | One-task, read-only Codex subscription execution with exact-scope patch application | Accepted implementation boundary; target product-interface role superseded by 0016 |
| [0015](0015-native-cli-release-archives.md) | Versioned native CLI archives with checksums and quota-free installed-binary acceptance | Accepted |
| [0016](0016-project-supervisor-cli.md) | Project-supervisor CLI and project-first operating model | Accepted |

ADRs describe decisions, not implementation completion. A changed decision is recorded by a new ADR that supersedes the old one.
