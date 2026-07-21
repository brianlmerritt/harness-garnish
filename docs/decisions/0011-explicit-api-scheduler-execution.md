# ADR 0011: Explicitly pinned paid API scheduler execution

- Status: accepted; response-only limitation partially superseded by ADR 0012
- Date: 2026-07-21
- Supersedes: the scheduler activation boundary in ADR 0010

## Context

ADR 0010 kept the direct Rustls transport out of the normal scheduler while exact request plans, retry-aware reservations, dispatch accounting, and restart behaviour were established independently. Those boundaries now exist, but activating the transport solely because a project budget or credential reference exists would conflate configuration with current consent to spend. The scheduler also increments a task version when it atomically claims the task, while the reserved request digest must remain bound to the pre-claim canonical version.

The direct API adapter currently has no approved repository-write tool. Provider response content must not be treated as a patch or trusted verification result.

## Decision

The scheduler may construct the live direct API transport only when all of these gates pass:

- the daemon is started with `--execute-api` and the exact literal acknowledgement `I_ACCEPT_PAID_API_TASK_EXECUTION`;
- the configured scheduler candidate is the literal `api` adapter with provider `openai` or `anthropic`;
- the task has an exact `api:provider:account` pin, including when API is the scheduler's only candidate;
- the task is risk class 0 because this first scheduler executor exposes no repository-write tool;
- the latest request plan is enabled, binds the claim's recorded pre-transition task version and exact request digest, and uses the `implementer` role;
- effective session policy, the latest project budget, price evidence, model/role limits, concurrency, and full retry-aware currency/token/request reservation all pass before claim; and
- the claim, run, request plan, and reservation identities still match before secret resolution and dispatch.

The acknowledgement enables API policy only for the named API providers in that daemon invocation. It is not durable policy, cannot be inferred from a credential or budget, and cannot cause subscription-quota fallback.

Successful provider response content is neither persisted nor applied to the task worktree. The run manifest records an honest `host-direct-api` attestation with no writable mounts, the fixed provider network identity, and host-process secret resolution; it does not claim fake-container isolation. Only bounded response metadata, hashed request identity, usage, spend, and attempt evidence are durable. A completed response proceeds to the task's predeclared independent command verifier against the unchanged isolated worktree. Tool calls, refusals, truncation, and paused responses fail the run after accounting is settled.

A terminal provider failure fails the task. Transport or authoritative-response uncertainty fails the task, retains the dispatched reservation, and cannot be replayed automatically after restart. Normal tests inject fake transports and never construct the network-enabled path.

## Consequences

- Starting a daemon with the acknowledgement can make multiple paid requests, bounded by exact task pins, request plans, scheduler limits, and project budgets. It is not the one-request smoke-test acknowledgement.
- Claim-time task state changes do not invalidate or silently regenerate the already-reserved request body.
- API tasks that require repository changes are denied before claim through `api.execution_tools_unavailable`; they cannot succeed through model text alone. A later controlled built-in tool or reviewed MCP/ACP boundary must define deterministic application, sandboxing, evidence, and approval rules.
- The separately ignored paid smoke test remains the narrow one-request transport diagnostic.
- Portability and normal suites remain fixture-only and quota-free.
