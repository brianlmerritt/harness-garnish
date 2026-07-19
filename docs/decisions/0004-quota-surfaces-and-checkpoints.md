# ADR 0004: Multi-surface quota policy and checkpoints

- Status: accepted
- Date: 2026-07-19

## Decision

Represent each provider/account/model entitlement as zero or more quota surfaces. A surface may be a five-hour, weekly, monthly, credit, monetary, request, token, or provider-specific window. Store provider-reported, user-overridden, inferred, and unknown values with provenance and confidence; never collapse them into an authoritative single percentage.

Users may adjust the effective remaining percentage, reserve, reset, confidence, or paid-overage policy while a project is running. Changes are append-only policy/quota events. The scheduler re-evaluates future phases immediately; the active phase checks at the next safe checkpoint and may pause sooner if a hard boundary is crossed.

Default reserve is 20% per applicable surface. Default checkpoint interval is five minutes, shortened when estimated time-to-boundary or user policy requires it. Unknown authoritative quota fails closed for unattended subscription work by default.

## Consequences

- Routing remains honest about uncertainty and varied subscriptions.
- Estimation must use historical distributions, not deterministic token arithmetic.
- A manual override does not rewrite source data and always records actor, reason, scope, and expiry.

