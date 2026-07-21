# Phase 4 live Codex subscription checkpoint

- Date: 2026-07-21
- Platform: macOS host
- Codex CLI: 0.144.6
- Scope: separately opted-in one-task subscription patch smoke
- Result: passed after one compatibility regression and a new explicit task

## Executed boundary

The passing run submitted exactly one acknowledged Codex subscription task with zero automatic retries and no paid-API fallback. Codex received a temporary Git checkout under the built-in `:read-only` permission profile and returned a patch constrained to `result.txt`. Garnish applied the patch only to its isolated task worktree, then a separate detached worktree verified that `result.txt` contained exactly `done`. The registered source checkout remained unchanged.

The test completed in 12.67 seconds. It emitted a private mode-0600 redacted receipt only after the exact ignored Cargo test passed. The receipt records one task, zero retries, exact scope, detached verification, unchanged source checkout, absence of persisted raw model output, Codex CLI version, and timestamp. It contains no authentication, prompt, model output, private reasoning, or quota percentage and remains outside version control under `target/codex-smoke-receipts/`.

## Compatibility regression

The first separately acknowledged task completed at the Codex process boundary but was rejected before patch application because Garnish treated more than one completed `agent_message` as ambiguous. Codex 0.144.6 can emit non-patch progress messages before its final patch response. No passing receipt was written for that attempt.

The quota-free regression permits bounded non-patch progress messages while requiring the last agent message to be the sole patch-bearing message. It continues to reject multiple patch candidates, a patch followed by prose, incomplete or ambiguous lifecycle events, prohibited capability events, malformed JSONL, non-patch final output, and oversized output.

## Evidence boundary

The passing private receipt was inspected locally and the quota-free suite subsequently passed 159 tests with 5 explicitly external tests ignored. Formatting, strict Clippy, shell syntax, and diff checks passed. The normal suite removed all Codex/API acknowledgements and credential selectors and made no provider request.

This is machine evidence for the narrow Codex 0.144.6 macOS subscription patch boundary. It is not proof of broader model quality, cross-platform conformance, production readiness, executable MCP safety, or Phase 4 exit.
