# Archived: narrow CLI MVP boundary

Status: archived on 2026-07-21. This document preserves the implemented and tested narrow CLI boundary as historical evidence. It is not the intended Harness Garnish product interface and must not be used as the current product roadmap.

The authoritative future operator and CLI contract is [`../cli-bible.md`](../cli-bible.md). Archiving this document does not invalidate the associated exit reports, platform evidence, live smoke results, ADRs, or release-package acceptance.

## Historical boundary

The first user-facing CLI target was intentionally narrow: local project/task control plus implementation through either a Codex subscription or an explicitly budgeted OpenAI/Anthropic API. Other coding CLIs and agent orchestration systems were deferred.

### Included

- durable local projects, tasks, dependencies, schedules, pins, quota evidence, claims, leases, cancellation, recovery, notifications, diagnostics, and private backups;
- exact task creation with risk, scope, acceptance, and deterministic verification declared before execution, plus `task readiness` using the scheduler's actual candidate filters without claiming work;
- one-task Codex subscription patch execution under ADR 0014;
- separately acknowledged, explicitly pinned OpenAI/Anthropic API response and exact-scope patch execution under ADRs 0011 and 0012;
- detached command verification and `task review TASK_ID`, including manifest, patch digest/path, verification, handoff, and an explicit statement that integration is not authorized;
- an authenticated read-only loopback dashboard over canonical state;
- default-deny MCP server registration without MCP execution under ADR 0013.

### Excluded

- Claude Code, Antigravity, AoE, or arbitrary CLI execution;
- executable MCP lifecycle, discovery, installation, tool calls, secret resolution, skill attachment, ACP, or remote approval;
- browser-based task mutation, semantic agent review, automatic commit/merge/push/deploy, packaging, or production-readiness claims;
- automatic subscription-to-API fallback or shared consent between the two execution modes.

### Acceptance evidence

The quota-free suite had to prove exact pinning, one-task acknowledgement, scrubbed Codex invocation, bounded JSONL parsing, prohibited-extension rejection, exact-scope patch application to an isolated worktree, independent verification, stable review JSON, and absence of reasoning/secret canaries from durable artifacts. API fixture tests retained the equivalent budget, dispatch, retry/uncertainty, patch, verifier, and canary requirements.

The 2026-07-21 macOS quota-free suite passed 159 tests with 5 explicitly ignored external tests; formatting and strict Clippy passed. The operator-reported OpenAI and Anthropic response/patch smokes are recorded separately. The first opted-in Codex 0.144.6 smoke exposed and was rejected by an overly strict multiple-message parser before patch application. After the quota-free parser regression, a second explicitly acknowledged one-task smoke passed exact-scope application and detached verification and emitted the required private redacted receipt. Normal tests never perform that task.

This established the narrow CLI boundary described above: local project/task control, Codex subscription patch execution, explicitly budgeted API execution, deterministic verification, and review handoff were usable through the CLI. It did not establish Phase 4 exit, production readiness, browser mutation parity, executable MCP, support for additional coding CLIs, or live Codex execution on every supported platform.

Operator documentation required an explicit `Placeholders:` declaration before every copyable console block. A quota-free repository check enforced the declaration, and both CI and the Linux/WSL portability script ran it. The portability environment scrub included every API and Codex live selector, acknowledgement, receipt override, and real-container selector. The final committed-tree revalidation passed 160 tests with 5 external tests ignored on macOS, native Linux, and WSL2. This established cross-platform fixture conformance for the narrow CLI boundary.

The operator subsequently reported Codex CLI 0.144.6 and `Logged in using ChatGPT` from both the native-Linux and WSL2 hosts. This established compatible installation and subscription-authentication readiness without recording an account identifier, token, auth file, or quota value. Login status did not submit an agent task and was not live execution evidence; a separately acknowledged one-task smoke was therefore still required on each host.

The native-Linux live smoke subsequently passed. The first WSL2 live attempt exited locally with status 127 before clean Codex JSONL because its env-based Node launcher could not find `node` in Garnish's deliberately scrubbed path. Garnish classified the attempt as uncertain, wrote no passing receipt, and did not retry. The launch boundary then resolved that packaging shape to an exact absolute Node runtime plus absolute Codex script without inheriting the user's path. A new explicitly acknowledged WSL2 task passed exact-scope application and detached verification. The narrow live Codex boundary is evidenced on macOS, native Linux, and WSL2, and the formal result and residual scope are recorded in [`../cli-mvp-exit-report.md`](../cli-mvp-exit-report.md).
