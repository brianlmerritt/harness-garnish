# Phase 4 plan — budgeted API agents and controlled extensions

## Objective

Phase 4 adds direct OpenAI and Anthropic agent providers without allowing low subscription quota to become implicit paid API spend. API access is disabled by default at both effective-policy and project levels. Enabling a project requires a bounded period, an explicit provider/account and secret reference, at least one hard monetary/token/request ceiling, model/tool/role allowlists, per-request output and retry ceilings, and durable reservation/settlement evidence.

OpenAI targets the Responses API contract. The current official contract returns a response ID, terminal status, model, typed output items, and input/output/total-token usage; streaming uses typed events and may report usage only on the completed response. Harness Garnish therefore reserves the maximum allowed request before network access and settles only from bounded provider evidence. Anthropic follows the equivalent provider-specific adapter contract. Model IDs and prices remain configuration/evidence, not schema constants.

Normal tests use fake transports and fixtures. No test reads a real API key, reaches a provider, or consumes API credit unless it is separately labelled and the user explicitly opts in.

## Implementation stages

1. API budget state: project/provider enablement, bounded periods, secret references, allowed models/tools/roles, output/retry/concurrency ceilings, and append-only configuration evidence.
2. Atomic accounting: reserve worst-case currency/tokens/requests before dispatch; prevent concurrent overcommit; settle actual provider usage once; release failures/cancellation/orphans once.
3. Provider contracts: strict OpenAI Responses and Anthropic response/stream fixture parsers, request IDs, usage, cancellation, rate-limit classification, bounded output, and schema-drift failure.
4. Secret delivery and opt-in smoke tests: protected references only, no secret persistence/logging, explicit disclosed provider/model/budget/network effects, and no automatic CLI-to-API fallback.
5. Routing integration: API candidates pass the same capability, health, policy, schedule, role, and independent-verifier gates plus API-specific budget gates.
6. Controlled extensions: mutually authenticated SSH/Tailscale approval transport, bounded MCP server lifecycle/tool allowlists, reviewed skill attachment, and ACP event mapping without transferring authority to those protocols.
7. Operator interface: read-only API budget/reservation/spend state first; mutations only after CLI parity, CSRF protection, explicit confirmation, and durable action evidence.

## Machine acceptance

| ID | Assertion |
| --- | --- |
| P4-01 | With no enabled project budget, every OpenAI/Anthropic reservation fails before secret access or transport invocation. |
| P4-02 | Enabling requires provider, non-secret secret reference, bounded period, allowed model/role, per-request maximum, reason, and at least one hard currency/token/request ceiling. |
| P4-03 | Subscription quota and API budget remain distinct resources; low subscription quota never creates or selects a paid API route automatically. |
| P4-04 | Monetary values use integer minor/micro units with an explicit currency; no binary floating-point arithmetic decides admission. |
| P4-05 | Concurrent reservations atomically include committed spend plus outstanding reservations and cannot exceed any configured ceiling. |
| P4-06 | A reservation is bound to project/task/provider/model/request digest and is single-settlement; replay or mismatched settlement fails closed. |
| P4-07 | Completion records provider request-ID hash, token categories, actual/estimated cost provenance, and releases unused reservation without storing prompts, output, or secrets. |
| P4-08 | Cancellation, failure, timeout, restart, and orphan recovery release or retain accounting exactly once according to whether provider dispatch was claimed. |
| P4-09 | Model, tool, role, output, retry, concurrency, network, and secret grants are hard filters recorded with stable reason codes. |
| P4-10 | OpenAI Responses fixtures accept documented typed output/usage and completed streaming events, tolerate additive fields, and reject missing, ambiguous, malformed, partial, or drifting required fields. |
| P4-11 | Anthropic fixtures meet the same bounded request-ID, terminal-state, usage, drift, and error-classification contract. |
| P4-12 | Provider rate/usage-limit failures remain distinct from authentication, transient server, policy, and local-budget failures; retries remain bounded and budget-reserved. |
| P4-13 | API keys never enter SQLite, argv, project files, logs, events, patches, fixtures, support bundles, or error messages; canary-secret tests cover every artifact path. |
| P4-14 | Normal macOS/Linux/WSL2 suites use fake transports only and consume no provider subscription quota or paid API credit. |
| P4-15 | Remote approval, MCP, skill, and ACP inputs cannot bypass core policy, action-digest approvals, quota/budget gates, sandbox attestation, or independent verification. |

## Intermediate status

Schema 15 now implements network-free project API budgets and atomic reservations/settlement. Stable CLI JSON plus fake-only default-deny, validation, concurrent overcommit, restart recovery, dispatch, settlement, replay, migration-backup, and request-exhaustion tests pass on macOS. After adding the provider fixture contracts, the 2026-07-20 normal suite passed 115 tests with the two explicitly opt-in real-container tests ignored; it made no provider request and consumed no subscription quota or API credit.

The next network-free slice adds strict OpenAI Responses and Anthropic Messages response/stream fixture parsers. They accept additive non-authoritative fields, require bounded typed output, terminal state, provider request identity, and exact usage, and reject malformed, partial, reordered, ambiguous, or authoritative schema-drift cases. Error fixtures keep authentication, permission, rate limits, paid-usage exhaustion, invalid requests, and transient provider failures distinct. There is still no HTTP transport or secret access, so this slice cannot spend money.

The protected-reference slice centralizes exact `env:NAME`, `file:/absolute/path`, and macOS `keychain:SERVICE/ACCOUNT` locators. Secret values are non-cloneable and non-serializable, have redacted debug output, receive bounded reads, and are cleared on drop. Unix files require the current user, a regular non-symlink file, and no group/other permissions; macOS Keychain lookup uses bounded supervised host execution with locator-only argv. Canary tests cover errors, debug output, SQLite/WAL state, verified backups, diagnostics, and the authenticated UI snapshot. Its macOS suite passed 119 tests with the two real-container tests explicitly ignored. No normal test reads a real credential.

The request-boundary slice constructs only the fixed OpenAI Responses and Anthropic Messages HTTPS endpoints. It repeats active-period, provider, model, tool, and output-ceiling checks before resolving a secret, bounds instructions/input/tools/body, and keeps prompts, outputs, bodies, raw response/request IDs, and authorization material out of `Debug` and serialization. A transport trait receives sensitive parts only through an explicitly named closure; the crate does not yet provide a real HTTP implementation. Fake transport fixtures prove request shapes, header placement, response parsing, and redacted provider failures without network access. Service preparation requires effective-policy permission, a live undispatched reservation, exact provider/model/output/request digest, and the still-latest budget revision. The macOS suite passes 123 tests with the two real-container tests explicitly ignored.

Schema 16 adds append-only provider/account/model/currency price evidence and preserves uncached input, cache-read input, cache-creation input, and output categories. OpenAI's documented `usage.input_tokens_details.cached_tokens` and `cache_write_tokens` meanings are treated as usage evidence, not pricing constants ([official prompt-caching contract](https://developers.openai.com/api/docs/guides/prompt-caching#requirements)). Rates are user-supplied integer currency micros per million tokens; no provider model, price, or cache multiplier is a program constant. Cost is calculated with checked integer arithmetic and a single final upward rounding. Monetary settlement requires the exact price record, verifies its identity and effective interval, recomputes cost, and remains single-use.

The transport-agnostic execution boundary is now complete for fake transports: exact request preparation precedes the dispatch claim; authoritative fixture usage selects the already-verified effective price; the raw provider request ID is hashed; and the reservation settles once. A canary test verifies that secret, prompt, instructions, response text, and raw request ID do not enter SQLite or a verified backup. This is machine evidence for the local accounting lifecycle, not authorization for real network traffic. The crate still has no real HTTP transport.

The 2026-07-20 macOS quota-free suite passed 129 tests with 2 opt-in real-container tests ignored. Strict Clippy with warnings denied also passed. No provider transport, credential, subscription quota, or paid API credit was used.

This is intermediate evidence, not Phase 4 exit. Schema 16 passed the quota-free macOS, native-Linux, and WSL2 suites: 129 tests passed on each platform, with the same 2 explicitly opt-in real-container tests ignored. Exact kernels, toolchains, and safety conditions are recorded in [`phase-4-portability-checkpoint.md`](phase-4-portability-checkpoint.md).

The first routing-integration slice gives the literal `api` adapter its own project-budget capacity gate instead of treating paid capacity as subscription percentage. It requires effective provider policy, the latest enabled and active project budget, implementer-role permission, concurrency and integer currency/token/request headroom, and effective price evidence for at least one allowed model. A mixed subscription/API candidate set marks the API lane `api.explicit_selection_required`; low subscription quota can never select it as fallback. An exact API pin reaches these gates, but automated scheduler claim is intentionally denied as `api.scheduler_execution_unavailable` until the scheduler can atomically bind an exact request digest and API budget reservation. The macOS quota-free suite now passes 132 tests with 2 opt-in real-container tests ignored; strict Clippy also passes.

## Explicit non-goals

Phase 4 does not enable API access merely because an environment variable exists, infer consent from an old prepaid balance, hard-code a current model alias or price, send a real request during normal tests, auto-install skills/MCP servers, broaden the local UI bind address, or permit remote approval to become an unbounded reusable authority. Packaging, signed self-update activation, Apple Container general support, encrypted portable export, and automatic Git integration remain later work.
