# ADR 0018: Implement TB-1 as a project-first fixture supervisor on schema 21

Status: Accepted

## Context

ADR 0017 froze a project-first command and state contract. TB-1 needs an executable vertical slice without prematurely authorising subscription use, paid APIs, credential projection, permission brokering, or container execution. The schema plan assigned some final settings, handoff, review, and cleanup structures to later versions, but TB-1 needs bounded evidence for those concepts now.

## Decision

Schema 21 adds canonical project supervision, `work|non_work|both` affinity, revisioned `W|O|B` calendars, local objectives, deterministic objective-to-task linkage, and inert migration from schema 20.

It also adds deliberately narrow TB-1 records for global setting explanations, two immutable fixture agent profiles, cross-profile handoff evidence, review results, cleanup ownership/outcomes, and supervisor cycles. These rows are real canonical TB-1 state, but their allowed modes and values are restricted to fixture execution. Schema 22–24 will extend or supersede their constraints; their presence does not claim the later approval, notification, secret, sandbox-attestation, or real-agent contracts.

The foreground service scans active projects in deterministic order, evaluates their project calendar, selects fake Codex when capacity permits, otherwise tries fake Claude, generates review evidence through the established isolated-worktree/verifier boundary, and automatically removes only resources whose exact Garnish ownership it proves. Results remain outside the source checkout until explicit `project apply`; integration and commits remain user-controlled.

The canonical compatibility route is `advanced`. Legacy top-level parser aliases remain hidden temporarily so existing automation is not broken before the announced compatibility removal boundary.

## Consequences

- Operators can exercise a useful project/objective workflow without seeing task IDs.
- TB-1 acceptance cannot consume subscription/API quota or present fake execution as real Codex/Claude evidence.
- A no-op objective is valid, while explicit `--fixture-write-path` and `--fixture-write-content` controls allow a visible quota-free patch exercise.
- Upgraded projects never become active automatically.
- Settings are revisioned, but only the TB-1 global/calendar subset is effective; complete layering remains TB-2.
- The service has a continuous foreground form and a bounded `--max-cycles` diagnostic form. OS service installation remains TB-6.
