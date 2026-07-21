# ADR 0012: Controlled API patch submission

- Status: accepted
- Date: 2026-07-21
- Supersedes: the response-only limitation in ADR 0011 for explicitly opted-in patch tasks

## Context

ADR 0011 deliberately limited paid scheduler execution to risk-class 0 responses. A useful provider test for implementation work must prove that a model can propose a repository change, that Garnish can confine it to the declared task scope, and that a separate verifier can evaluate it. Giving the provider a shell, filesystem, general tool protocol, or source-checkout access would broaden authority unnecessarily.

## Decision

Garnish adds one built-in API tool named `submit_patch`. It is available only when every response-only gate in ADR 0011 passes and all of these additional gates pass:

- the task is risk class 1 and explicitly requires `agent.patch_submission`;
- its current API budget explicitly allows `submit_patch`;
- the daemon has both the paid execution acknowledgement and `--execute-api-patches --acknowledge-api-patches I_ACCEPT_ISOLATED_API_PATCH_EXECUTION`;
- task scope contains only exact, safe repository-relative paths; and
- the provider returns exactly one completed `submit_patch` call with exactly one string field named `patch`.

The provider receives no shell, filesystem, network, MCP, ACP, or arbitrary function tool. Garnish caps the UTF-8 patch at 1 MiB and rejects empty or malformed diffs, NUL or binary patches, symlink and submodule modes, renames, copies, extra tool calls, and unexpected arguments. It runs `git apply --check` before application and applies only to the clean isolated task worktree. After application, every changed file must exactly match a declared scope path and must not be a symbolic link. A rejection fails the run and leaves any change confined to that quarantined worktree; it never changes the source checkout.

This deterministic control-plane application is a narrow exception to the general secure-container rule for class-1 agent writes: the untrusted provider never executes and never holds write access. Policy must separately allow isolated branch changes. Risk classes 2 and 3 still require approval and cannot use this boundary.

The accepted patch becomes the normal run patch artifact. Raw provider response bodies, request IDs, and prose are not persisted. The manifest truthfully records `host-direct-api`, no secure container, and the isolated worktree as the only writable mount mediated by the control plane. A separate detached verifier worktree receives the resulting patch and runs only the task's predeclared verification argv.

Normal tests use fake OpenAI and Anthropic transports. Any real-provider patch smoke test must be separately ignored, disclose that it can spend one paid request, use a temporary repository and state directory, reserve exactly one request with no retries, and require its own literal one-request acknowledgement.

## Consequences

- A budget, credential, task pin, or paid-daemon acknowledgement alone cannot authorize repository changes.
- Model text is never interpreted as a patch; only the single typed tool result is authoritative.
- Out-of-scope output may dirty only the isolated worktree before the post-apply scope check rejects it. The source checkout remains unchanged and the rejected worktree is retained for diagnosis.
- The attestation does not claim container isolation because none exists at this boundary.
- General tool execution, shell access, semantic review, automatic integration, and risk-class 2/3 effects remain out of scope.
