# Phase 4 live Codex subscription checkpoint

- Date: 2026-07-21
- Platforms: macOS, native Linux, and WSL2
- Codex CLI: 0.144.6
- Scope: separately opted-in one-task subscription patch smoke
- Result: passed on all three platforms after bounded macOS parser and WSL2 launcher regressions

## Executed boundary

The passing run submitted exactly one acknowledged Codex subscription task with zero automatic retries and no paid-API fallback. Codex received a temporary Git checkout under the built-in `:read-only` permission profile and returned a patch constrained to `result.txt`. Garnish applied the patch only to its isolated task worktree, then a separate detached worktree verified that `result.txt` contained exactly `done`. The registered source checkout remained unchanged.

Each passing test emitted a private mode-0600 redacted receipt only after the exact ignored Cargo test passed. The receipt records one task, zero retries, exact scope, detached verification, unchanged source checkout, absence of persisted raw model output, Codex CLI version, and timestamp. It contains no authentication, prompt, model output, private reasoning, or quota percentage and remains outside version control under `target/codex-smoke-receipts/`.

| Platform | Codex packaging | Passing duration | Result |
| --- | --- | ---: | --- |
| macOS | standalone application binary | 12.67 seconds | Passed; private receipt emitted |
| Native Linux | standalone CLI | 15.84 seconds | Passed; private receipt emitted |
| WSL2 | NVM Node launcher | 12.43 seconds | Passed; private receipt emitted |

## Compatibility regression

The first separately acknowledged task completed at the Codex process boundary but was rejected before patch application because Garnish treated more than one completed `agent_message` as ambiguous. Codex 0.144.6 can emit non-patch progress messages before its final patch response. No passing receipt was written for that attempt.

The quota-free regression permits bounded non-patch progress messages while requiring the last agent message to be the sole patch-bearing message. It continues to reject multiple patch candidates, a patch followed by prose, incomplete or ambiguous lifecycle events, prohibited capability events, malformed JSONL, non-patch final output, and oversized output.

The first WSL2 attempt exited locally with status 127 in 0.55 seconds because the installed Codex path used `#!/usr/bin/env node` while Garnish deliberately removed the NVM directory from the child search path. Garnish conservatively classified the attempt as uncertain, wrote no passing receipt, applied no patch, and did not retry. A quota-free diagnostic proved that bubblewrap and the built-in `:read-only` profile worked and that exact absolute Node plus absolute Codex script execution also worked under the scrubbed environment. Garnish now recognizes only that env-based Node launcher shape, resolves the installed Node executable once, and invokes both absolute paths without restoring the inherited path. The focused WSL2 regression passed before a new explicitly acknowledged task was submitted.

## Evidence boundary

The macOS private receipt was inspected locally; the native-Linux and WSL2 terminal evidence reported their private receipt paths after the exact ignored test passed. The quota-free suite passed 159 tests with 5 explicitly external tests ignored on macOS, native Linux, and WSL2 before the launcher regression. After the exact-runtime fix, the full suite passed 160 tests with the same 5 ignored tests on all three platforms. Formatting, strict Clippy, shell syntax, and command-placeholder checks passed. Normal suites removed live acknowledgements and made no provider request.

This is machine evidence for the narrow Codex 0.144.6 subscription patch boundary on macOS, native Linux, and WSL2. It is not proof of broader model quality, production readiness, executable MCP safety, or Phase 4 exit.
