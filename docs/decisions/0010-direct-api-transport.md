# ADR 0010: Direct API transport without hidden replay

- Status: accepted; the scheduler activation boundary is superseded by ADR 0011
- Date: 2026-07-20

## Context

Schema 19 durably accounts for every provider dispatch attempt and permits retries only after an explicitly classified retryable provider response. A general-purpose HTTP client can silently follow redirects, inherit proxy settings, or replay a request after a protocol failure. Any of those behaviours could disclose a credential or create paid provider work that is absent from Garnish's attempt journal.

## Decision

Use a blocking Reqwest client with Rustls and the Ring crypto provider for direct OpenAI Responses and Anthropic Messages calls. The client:

- accepts only the two compiled HTTPS endpoints;
- follows no redirects and sends no referrer;
- ignores environment proxy configuration;
- disables all client-library retries, including protocol-NACK retries;
- applies bounded connect, request, response-size, content-type, and provider-request-ID checks;
- marks the authorization header sensitive and never includes secrets or bodies in its own debug representation; and
- remains disabled unless its caller supplies an explicit per-construction network opt-in.

The normal CLI, scheduler, and test suite do not construct this transport. The first executable entry point is a separately ignored, one-request paid smoke test behind `scripts/test-real-api-smoke`. It requires an exact provider, exact model ID, protected secret reference, and literal paid-request acknowledgement. It has one reserved request, no retries, token ceilings, a temporary database, and no configured currency price evidence.

## Consequences

- A transport call maps one-to-one to a durable Garnish dispatch attempt.
- Corporate proxies are unsupported until a separately reviewed proxy grant can preserve endpoint, credential, and attempt-accounting guarantees.
- The smoke test can consume paid API credit and is never run by normal or portability suites.
- A timeout or other post-dispatch uncertainty is retained and cannot be replayed automatically.
- Ring-backed Rustls avoids a new OpenSSL, AWS-LC, or CMake system-package requirement on macOS, native Linux, and WSL2.

## Contract sources

- [OpenAI Responses create contract](https://developers.openai.com/api/reference/resources/responses/methods/create)
- [OpenAI request-ID headers](https://developers.openai.com/api/reference/overview#debugging-requests)
- [Anthropic direct API authentication](https://platform.claude.com/docs/en/manage-claude/authentication)
- [Anthropic errors and request-ID header](https://platform.claude.com/docs/en/api/errors)
