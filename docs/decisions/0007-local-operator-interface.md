# ADR 0007: Local operator interface

- Status: accepted
- Date: 2026-07-20

## Decision

Keep the CLI as the canonical scriptable interface and add a local web interface for human operation. The first slice is read-only and renders the same durable SQLite state exposed by service methods and a versioned local JSON snapshot endpoint.

Bind only to IPv4 loopback. Each server start generates a cryptographically random, ephemeral capability token. The one-time startup URL exchanges that token for an `HttpOnly`, `SameSite=Strict` cookie and immediately redirects to a clean URL. Requests must also pass strict `Host` validation; framing, cross-origin access, caching, request bodies, and non-GET methods are denied in the first slice.

Mutation controls will be added only with explicit confirmation, CSRF protection, bounded typed inputs, policy authorization, and durable evidence equivalent to the CLI. Remote access through SSH or Tailscale is a later adapter around the local service, not a broader default bind.

## Consequences

- macOS, Linux, and WSL2 share one browser-based operator experience while VPS operation remains CLI-friendly.
- “Why is this not running?” is derived from durable scheduler reason codes rather than inferred UI text.
- The ephemeral URL is sensitive for the life of the UI process and must not be copied into logs or project files.
- A future desktop wrapper can reuse the local service without becoming canonical state.
