# Phase 3 exit report

## Result

Phase 3 exited successfully on 2026-07-20. Schema 14 multi-agent capability and quota routing passed its normal quota-free suite on macOS, a Linux VPS, and Ubuntu 24.04 under WSL2. This establishes deterministic routing, quota evidence/reservations/forecasting, the authenticated read-only operator interface, and independently attributable command-verifier runs. It is not a claim that real coding-agent execution or semantic agent review is enabled.

## Final platform evidence

### macOS development host

- Formatting, strict Clippy, and `git diff --check` completed without warnings or errors.
- Rust library suite: 96 passed, 0 failed, 2 explicitly ignored opt-in real-container tests.
- CLI integration suite: 6 passed, 0 failed.
- CLI parser suite: 1 passed, 0 failed.
- MVP vertical slice: 1 passed, 0 failed.
- Total: 104 passed, 0 failed, 2 explicitly ignored.
- Authenticated AoE/UI loopback, quota parser drift, append-only usage history, P90 fallback, schema-12/13 migrations, independent verifier selection, clean verifier worktree, and orphan recovery passed.
- Explicit real, non-task capability probes previously recorded healthy Codex CLI `0.144.6`, Claude Code `2.1.215`, and Antigravity `1.1.4` evidence.
- Explicit CodexBar collection previously passed for Codex OAuth and Claude CLI sources without submitting an agent task; missing or failed collection remained distinct durable evidence.

### Linux VPS

- Kernel: Linux `6.8.0-107-generic` on `x86_64`.
- Rust and Cargo: `1.97.1`.
- Rust library suite: 96 passed, 0 failed, 2 explicitly ignored opt-in real-container tests.
- CLI integration suite: 6 passed, 0 failed; CLI parser and MVP vertical-slice suites each passed.
- Bounded daemon shutdown completed through `max_ticks`; signal-driven shutdown completed with `shutdown_reason: signal`.
- State directory and database modes were `0700` and `0600`.
- Rootless Podman and Docker runtime probes were healthy. Their digest-pinned conformance tests were not repeated because Phase 3 did not change the already-passed sandbox backend implementation.

### WSL2

- Distribution: Ubuntu 24.04; kernel `6.6.87.2-microsoft-standard-WSL2` on `x86_64`.
- Rust and Cargo: `1.97.1`.
- Checkout: Linux-native filesystem at `/home/blm/dev/docker/harness-garnish`.
- Rust library suite: 96 passed, 0 failed, 2 explicitly ignored opt-in real-container tests.
- CLI integration suite: 6 passed, 0 failed; CLI parser and MVP vertical-slice suites each passed.
- The explicit Windows-mounted-root denial test passed.
- Rootless Podman was healthy. With no systemd user session it emitted a warning and safely selected its `cgroupfs` fallback. Docker was not installed and was not required because a supported healthy runtime was present.
- State directory, database, and backup modes were `0700`, `0600`, and `0600`.
- The schema-14 online backup passed integrity checking and emitted SHA-256 `3245c637e90317dcdbb753a2f35f92338cd38a85ecb712ece5119b05907b19b2`.

No provider task quota or paid API budget was consumed by the macOS, Linux, or WSL2 acceptance suites.

## Acceptance mapping

| Acceptance | Final evidence |
| --- | --- |
| P3-01–P3-02 | Append-only Codex, Claude, and Antigravity probes retain bounded version/capability/health evidence; missing, stale, unsupported, unknown, and fixture drift remain distinct fail-closed states. |
| P3-03 | Multi-candidate ordering and lexical tie-breaking pass deterministic tests; route events retain every hard filter and score component. |
| P3-04 | Durable exact manual pins cannot bypass capability, health, policy, freshness, or quota gates. |
| P3-05–P3-06 | Malformed, stale, unknown, conflicting, and missing quota lanes remain distinct; five-hour, weekly, monthly, and paid-extra surfaces remain independent. |
| P3-07 | Immediate transactions prevent concurrent reservation overcommit and release an expired/orphaned reservation once. |
| P3-08 | Live quota overrides and pin changes create new decisions without rewriting prior route evidence. |
| P3-09 | Schema 14 creates a separately selected verifier route and child run with a clean detached worktree and distinct evidence. Default policy requires a different adapter and can require a different provider. |
| P3-10 | Normal suites on all three platforms used fake providers and local command verification only; no provider task or API request was issued. |
| P3-11 | The operator interface passed loopback binding, token-to-cookie authentication, exact Host validation, unauthenticated rejection, and durable queue-reason rendering tests. |
| P3-12 | Schema 13 usage samples are append-only, replay-detecting, restart durable, and exact-identity scoped; bounded P90 begins only after five groups and sparse history retains the conservative fallback. |

## Residual scope

Real Codex, Claude Code, and Antigravity task execution remains disabled until each is connected through an attested secure-container runtime and supervised adapter lifecycle. The initial verifier executes declared commands and does not claim semantic agent review. OpenAI/Anthropic API agents and budgets, mutable web controls, remote access/notifications, Apple Container conformance, packaging, signed self-update, encrypted portable export, Skills/MCP/ACP lifecycle, and automatic Git integration remain later-phase work.
