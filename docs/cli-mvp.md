# CLI MVP boundary

The first user-facing CLI target is intentionally narrow: local project/task control plus implementation through either a Codex subscription or an explicitly budgeted OpenAI/Anthropic API. Other coding CLIs and agent orchestration systems are deferred.

## Included

- durable local projects, tasks, dependencies, schedules, pins, quota evidence, claims, leases, cancellation, recovery, notifications, diagnostics, and private backups;
- exact task creation with risk, scope, acceptance, and deterministic verification declared before execution, plus `task readiness` using the scheduler's actual candidate filters without claiming work;
- one-task Codex subscription patch execution under ADR 0014;
- separately acknowledged, explicitly pinned OpenAI/Anthropic API response and exact-scope patch execution under ADRs 0011 and 0012;
- detached command verification and `task review TASK_ID`, including manifest, patch digest/path, verification, handoff, and an explicit statement that integration is not authorized;
- an authenticated read-only loopback dashboard over canonical state;
- default-deny MCP server registration without MCP execution under ADR 0013.

## Excluded

- Claude Code, Antigravity, AoE, or arbitrary CLI execution;
- executable MCP lifecycle, discovery, installation, tool calls, secret resolution, skill attachment, ACP, or remote approval;
- browser-based task mutation, semantic agent review, automatic commit/merge/push/deploy, packaging, or production-readiness claims;
- automatic subscription-to-API fallback or shared consent between the two execution modes.

## Acceptance evidence

The quota-free suite must prove exact pinning, one-task acknowledgement, scrubbed Codex invocation, bounded JSONL parsing, prohibited-extension rejection, exact-scope patch application to an isolated worktree, independent verification, stable review JSON, and absence of reasoning/secret canaries from durable artifacts. API fixture tests retain the equivalent budget, dispatch, retry/uncertainty, patch, verifier, and canary requirements.

The 2026-07-21 macOS quota-free suite passed 159 tests with 5 explicitly ignored external tests; formatting and strict Clippy passed. The operator-reported OpenAI and Anthropic response/patch smokes are recorded separately. The first opted-in Codex 0.144.6 smoke exposed and was rejected by an overly strict multiple-message parser before patch application. After the quota-free parser regression, a second explicitly acknowledged one-task smoke passed exact-scope application and detached verification and emitted the required private redacted receipt. Normal tests never perform that task.

This establishes the narrow CLI MVP boundary described above: local project/task control, Codex subscription patch execution, explicitly budgeted API execution, deterministic verification, and review handoff are usable through the CLI. It does not establish Phase 4 exit, production readiness, browser mutation parity, executable MCP, support for additional coding CLIs, or live Codex execution on every supported platform.

Operator documentation now requires an explicit `Placeholders:` declaration before every copyable console block. A quota-free repository check enforces the declaration, and both CI and the Linux/WSL portability script run it. The portability environment scrub includes every API and Codex live selector, acknowledgement, receipt override, and real-container selector. At that checkpoint, the macOS, native-Linux, and WSL2 suites each passed 159 tests with 5 external tests ignored. This established cross-platform fixture conformance for the narrow CLI MVP; live Codex subscription execution had then been exercised only on macOS.

The operator subsequently reported Codex CLI 0.144.6 and `Logged in using ChatGPT` from both the native-Linux and WSL2 hosts. This established compatible installation and subscription-authentication readiness without recording an account identifier, token, auth file, or quota value. Login status does not submit an agent task and is not live execution evidence; a separately acknowledged one-task smoke was therefore still required on each host.

The native-Linux live smoke subsequently passed. The first WSL2 live attempt exited locally with status 127 before clean Codex JSONL because its env-based Node launcher could not find `node` in Garnish's deliberately scrubbed path. Garnish classified the attempt as uncertain, wrote no passing receipt, and did not retry. The launch boundary now resolves that packaging shape to an exact absolute Node runtime plus absolute Codex script without inheriting the user's path. The updated macOS quota-free suite passes 160 tests with 5 external tests ignored, the focused WSL2 regression passed, and a new explicitly acknowledged WSL2 task then passed exact-scope application and detached verification. The narrow live Codex boundary is now evidenced on macOS, native Linux, and WSL2.
