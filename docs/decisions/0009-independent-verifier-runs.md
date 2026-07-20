# ADR 0009: Independent verifier runs

- Status: accepted
- Date: 2026-07-20

## Decision

Represent deterministic verification as a separately selected verifier route and a distinct child run, never as an undocumented step performed by the implementation run. The verifier receives a clean detached worktree reconstructed from the implementation base plus patch, its own manifest, route, bounded process output, and verification evidence. A durable verification row links the implementer and verifier runs.

The verifier identity must always differ from the exact implementer identity. Default policy also requires a different adapter; project policy may additionally require a different provider. Candidate filtering uses stable reason codes and lexical identity ordering. The initial executable verifier is Garnish's local command verifier, which runs only the task's predeclared argv and consumes no provider quota. Agent-based review adapters can later enter the same selection contract after their quota and secure-execution gates are connected.

## Consequences

- A successful implementation process cannot mark its own work verified.
- Implementer and verifier evidence survive restart as independently attributable records.
- The current command verifier checks declared commands, not semantic code-review quality; a future agent reviewer remains a separate adapter, not an inflated claim about this runner.
- Failed verifier selection or execution cannot silently become a passed implementation run.
