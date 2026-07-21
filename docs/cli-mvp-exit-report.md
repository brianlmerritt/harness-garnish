# CLI MVP exit report

## Result

The narrow source-run CLI MVP exited successfully on 2026-07-21 at Git revision `747d59864eb98ab75206f0f02191d2e6f81628a3` (`working on mvp`). SQLite schema 20, the final quota-free fixture suite, the Codex subscription patch path, the explicitly budgeted OpenAI/Anthropic paths, detached verification, review projection, read-only loopback dashboard, and default-deny MCP registration are implemented and evidenced within the boundary defined by [`cli-mvp.md`](cli-mvp.md).

This result means a technically competent operator can build and use the MVP from its source checkout. It is not Phase 4 exit, production readiness, a packaged end-user release, browser mutation parity, executable MCP, or support for arbitrary coding-agent CLIs.

## Implemented scope

- Durable local projects, tasks, dependencies, schedules, pins, quota observations, claims, leases, cancellation, recovery, notifications, diagnostics, and private backups.
- Exact task contracts and scheduler-backed readiness inspection before execution.
- One-task Codex subscription patch execution with explicit acknowledgement, fresh supported capability evidence, quota headroom, read-only commands, prohibited extensions disabled, exact patch scope, and no automatic paid-API fallback.
- Separately acknowledged and exactly pinned OpenAI and Anthropic response-only and isolated-patch execution with project budgets, request/output/retry ceilings, protected secret references, durable accounting, and uncertain-request replay denial.
- Patch application only to an isolated task worktree, deterministic verification in a separate detached worktree, and stable `task review` evidence that does not authorize integration.
- An authenticated read-only loopback dashboard over canonical state.
- Append-only, default-deny MCP registration validation without launching a server or invoking a tool.

## Final quota-free platform evidence

All normal acceptance runs removed provider credentials, live selectors, acknowledgements, receipt overrides, and real-container selectors from the test process. They used fixtures and fake executables only and consumed no provider request, subscription task, or paid API credit.

| Platform | Runtime | Library | CLI parser | CLI integration | Vertical slice | External tests | Result |
| --- | --- | ---: | ---: | ---: | ---: | ---: | --- |
| macOS | Rust/Cargo 1.97.1 | 144 passed | 1 passed | 14 passed | 1 passed | 5 ignored | 160 passed, 0 failed |
| Native Linux | Linux 6.8.0-107-generic x86_64; Rust/Cargo 1.97.1 | 144 passed | 1 passed | 14 passed | 1 passed | 5 ignored | 160 passed, 0 failed |
| WSL2 | Linux 6.6.87.2-microsoft-standard-WSL2 x86_64; Rust/Cargo 1.97.1 | 144 passed | 1 passed | 14 passed | 1 passed | 5 ignored | 160 passed, 0 failed |

Formatting, strict Clippy, command-placeholder declarations, and the complete locked workspace suite passed. Native Linux and WSL2 emitted their respective `Phase 4 quota-free portability checkpoint passed` terminal markers. The five ignored tests remain separately opted-in external boundaries: two real-container checks and the live Codex, paid-API response, and paid-API patch smokes.

## Live execution evidence

- Codex CLI 0.144.6 completed one separately acknowledged, zero-retry, exact-scope subscription patch smoke on macOS, native Linux, and WSL2. Each passing run verified `result.txt` from a detached worktree, left the registered source checkout unchanged, and emitted a private redacted receipt. The macOS parser and WSL2 NVM launcher compatibility failures were fixed and covered by quota-free regressions before the successful replacement runs. Details are in [`phase-4-live-codex-checkpoint.md`](phase-4-live-codex-checkpoint.md).
- The operator reported successful one-request response-only and exact-scope patch smokes for both OpenAI and Anthropic, for four zero-retry requests in total. Those scripts predated durable receipt output, so [`phase-4-live-api-checkpoint.md`](phase-4-live-api-checkpoint.md) records this as operator-reported evidence rather than independently inspectable Phase 4 exit evidence.

No automatic subscription-to-API fallback was enabled or exercised.

## Security and authority boundary

- Provider traffic remains default deny and requires exact runtime acknowledgement; normal and portability tests cannot opt in accidentally.
- Secret values, raw provider responses, Codex JSONL, stderr, prompts, and private reasoning are excluded from durable state and passing receipts.
- Agent output cannot expand command, filesystem, patch, Git, MCP, network, retry, or budget authority.
- Repository Git integration remains user managed. The MVP applies and verifies isolated patches but does not commit, merge, push, deploy, or claim that review authorizes those operations.
- MCP is a registration boundary only. Server lifecycle, discovery, installation, tool calls, secret resolution, skills, ACP, and remote approval remain disabled.

## Deferred work

- Binary packaging, installation, upgrades, versioned release artifacts, and end-user operator documentation.
- Browser-based mutation with CSRF protection, confirmations, typed inputs, policy checks, and durable evidence.
- Executable MCP and other controlled-extension acceptance work.
- Claude Code, Antigravity, AoE, and arbitrary CLI task execution.
- Semantic agent review, automatic Git integration, remote approval, and production hardening.
- A broader Phase 4 exit report. The API evidence limitation and unfinished controlled extensions prevent this CLI milestone from being used as a Phase 4 completion claim.

## Conclusion

The repository now has an accepted CLI MVP for source-based local operation. The next practical product milestone is packaging plus concise operator workflows; executable MCP and additional agent systems remain later, separately accepted capabilities.

## Subsequent distributable slice

After this source-run exit, ADR 0015 added native versioned archives, checksum manifests, installation instructions, task-oriented operator instructions, and a quota-free acceptance script that executes the extracted binary. The macOS Apple Silicon package acceptance passes locally. Native-Linux and WSL2 package acceptance remain required before recording a complete initial packaging matrix; this later work does not alter the revision-specific source-run result above.
