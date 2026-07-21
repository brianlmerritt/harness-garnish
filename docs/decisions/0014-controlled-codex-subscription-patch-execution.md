# ADR 0014: Controlled Codex subscription patch execution

- Status: accepted
- Date: 2026-07-21

## Context

The CLI MVP needs a useful coding path for a user's existing Codex subscription as well as the separately budgeted OpenAI and Anthropic API paths. Passing subscription authentication into a task container would expose password-like host authentication state, while giving a host Codex process workspace-write authority would let model-directed commands bypass Garnish's exact task scope and independent verification.

The official [Codex non-interactive mode](https://learn.chatgpt.com/docs/non-interactive-mode) supports ephemeral `codex exec` runs and JSONL events while reusing saved CLI authentication. The official [Codex permissions](https://learn.chatgpt.com/docs/permissions) contract provides a built-in `:read-only` permission profile. These primitives are useful controls but do not themselves establish Garnish task authority.

## Decision

Garnish provides a distinct Codex subscription scheduler mode. A daemon invocation must contain exactly one `codex` route candidate, `--execute-codex`, the literal `I_ACCEPT_ONE_CODEX_SUBSCRIPTION_TASK`, and a global active-claim limit of one. The eligible task must have an exact `codex:codex:ACCOUNT` pin, fresh supported Codex capability evidence, eligible subscription quota evidence with forecast headroom, risk class 1, and at least one exact safe repository-relative scope path. Automatically collected quota evidence must still be fresh; a manual observation is an explicit durable operator assertion and must not be mistaken for a fresh provider observation. Codex and paid API execution cannot be enabled in the same daemon invocation. After one Codex task finishes, the daemon stops.

The host control process discovers the supported Codex executable and invokes `codex exec` with an ephemeral JSONL thread, strict configuration, no approval prompts, and `:read-only` command permissions. User configuration, exec-policy rule files, MCP servers, apps, hooks, local and remote plugins, multi-agent operation, web search, and inherited shell environment are disabled. Repository content and Codex project instructions remain untrusted model input; they cannot alter the process permissions or Garnish's deterministic output gate. The child receives only the host paths needed for saved Codex authentication plus a fixed non-secret runtime environment. When the selected Codex executable is an exact `/usr/bin/env node` launcher, Garnish resolves the installed `node` executable once and invokes that absolute runtime with the absolute Codex script path; it does not reintroduce the inherited `PATH`. Authentication is never copied into a worktree, database, run artifact, prompt, argument, or event.

Codex receives an isolated worktree as read-only context and must return exactly one final UTF-8 unified Git patch. Garnish retains neither JSONL content nor stderr: it records only bounded lengths, hashes, lifecycle classification, and redacted failure evidence. Bounded non-patch progress messages are permitted, but the last agent message must be the sole patch-bearing message. Garnish rejects incomplete or ambiguous lifecycle events, multiple patch candidates, a patch followed by prose, prohibited capability events, non-patch output, oversized output, and malformed or unsafe diffs. The deterministic control plane applies the patch to the isolated worktree, rechecks the exact changed paths, and runs the existing detached verifier. Neither Codex nor Garnish modifies the registered source checkout or performs Git integration.

The supervisor renews halfway through each checkpoint interval and re-evaluates cancellation, calendar, policy, and subscription quota within the task's configured checkpoint ceiling. The fenced database lease includes a five-second scheduling grace so a checkpoint does not expire at the same instant it is renewed. A timeout, signal, truncation, nonzero exit, malformed output, or rejected patch is terminal for that attempt and is never retried automatically. Subscription execution never falls back to a paid API route. API execution retains its independent project budget, exact request plan, credential reference, and acknowledgement boundaries.

## Consequences

- Saved Codex subscription authentication remains a host-side trust dependency, so this boundary honestly records that it is not a secure container.
- An env-based Codex package adds its exact host Node runtime to the trusted launch chain, but not to the child search path.
- Model-directed commands can inspect the isolated checkout but cannot write it or access Garnish's MCP registration state.
- Garnish, not the model, owns the only write step, exact-scope validation, verification, and review handoff.
- The one-task acknowledgement has literal effect and bounds accidental subscription consumption per daemon invocation.
- Normal tests use a fake Codex executable and consume no subscription quota. A separately opted-in Codex 0.144.6 fixture task passed on macOS after a compatibility regression established the bounded progress-message rule; this is narrow live evidence, not general or cross-platform conformance.
- Claude Code, Antigravity, AoE, executable MCP servers, and automatic Git integration remain outside this CLI MVP.
