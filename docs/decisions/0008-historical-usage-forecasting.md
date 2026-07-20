# ADR 0008: Historical usage forecasting

- Status: accepted
- Date: 2026-07-20

## Decision

Forecast quota consumption from append-only, explicit usage evidence scoped to the exact adapter, provider, and account. Never infer a run's consumption by subtracting two account-level remaining-quota observations: unrelated clients, concurrent Garnish runs, quota-window resets, and provider rounding make that delta ambiguous.

Each usage row records a stable evidence identifier, quota surface, task estimate, consumed percentage, source, confidence, and observation time. The evidence identifier groups surfaces from the same run or collector result and makes replay detectable. Only control-plane collector measurements, provider reports, and explicit user reports are accepted; untrusted agent prose is not telemetry.

Use at most the latest 50 matching evidence groups. Within a group, use the greatest predicted percentage across its surfaces. Once five groups exist, reserve the nearest-rank 90th percentile adjusted by the new task's uncertainty. Before then, retain the existing conservative duration-based fallback. Every route candidate records its forecast value, source, and sample count; the selected value is the one committed to quota reservations.

## Consequences

- Different adapters or accounts can receive different quota forecasts for the same task.
- Other account activity cannot silently masquerade as this run's consumption.
- Five samples are intentionally slow to establish and P90 is conservative; sparse histories do not reduce fallback headroom.
- A bad trusted collector or incorrect user report can affect later forecasts, so evidence remains append-only and attributable rather than silently corrected.
- Per-surface reservation values can be added later without changing the evidence contract; the current scalar forecast conservatively takes the greatest grouped surface prediction.
