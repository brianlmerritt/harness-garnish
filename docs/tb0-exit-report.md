# TB-0 contract and migration baseline exit report

## Result

TB-0 exited successfully on 2026-07-21 on branch `pivot_cli`, subject to the user's normal commit process. The project-supervisor direction, exact normal/advanced CLI grammar, stable JSON/exit contract, state machines, complete current-command/service disposition, staged schema-20 migration, closed design decisions, and quota-free executable contract fixtures are now present.

This is a contract-phase exit. It does not claim that the current `garnish` executable implements the new normal commands, schema 21–25, `WWWOOBB`, project objectives/supervision, automatic cleanup, structured approval broker, notification delivery, secret management, containerised Codex/Claude execution, or subscription switching. Those begin at TB-1 and later phases.

## Canonical evidence boundary

- [`cli-bible.md`](cli-bible.md) and [ADR 0016](decisions/0016-project-supervisor-cli.md): product direction;
- [`tb0-command-contract.md`](tb0-command-contract.md), [`contracts/tb0-cli-v1.json`](contracts/tb0-cli-v1.json), and [ADR 0017](decisions/0017-tb0-cli-contract.md): exact operator/machine contract and resolved implementation decisions;
- [`tb0-state-contracts.md`](tb0-state-contracts.md): lifecycle and transition invariants;
- [`tb0-gap-matrix.md`](tb0-gap-matrix.md): every current command family and source service mapped to reuse, replacement, or advanced compatibility;
- [`tb0-schema-migration.md`](tb0-schema-migration.md): schema 21–25 sequence, legacy mappings, inert activation, backup/rollback, and migration fixtures;
- [`tb0-acceptance.md`](tb0-acceptance.md) and `scripts/test-tb0-contract`: acceptance requirements and repeatable quota-free entry point;
- `tests/tb0_cli_contract.rs`: executable contract/parser assertions.

## Machine evidence

The quota-free `./scripts/test-tb0-contract` acceptance passed:

- command placeholder declarations: complete;
- `cargo fmt --all -- --check`: passed;
- focused `tb0_cli_contract`: 4 passed;
- strict workspace/all-target/all-feature Clippy with warnings denied: passed;
- library suite: 144 passed, 5 explicitly ignored external tests;
- binary suite: 1 passed;
- current CLI JSON suite: 14 passed;
- historical vertical slice: 1 passed;
- TB-0 contract suite within the workspace run: 4 passed;
- doc tests: 0 tests, passed.

The workspace total is 164 passing tests and 5 explicitly ignored external tests. Ignored tests are the two real-container checks and the three separately opted-in live Codex/API checks.

The first sandboxed full-suite attempt reached the existing AoE/UI loopback fixtures and received local `EPERM` bind errors. The identical quota-free script was rerun with permission to bind temporary loopback sockets and passed. This did not authorise or perform provider network access.

Every Cargo process ran with OpenAI/Anthropic credentials, live-test acknowledgements/selectors, receipt destinations, Garnish data-directory override, and real-container selectors removed. No provider request, subscription task, paid API call, real container, keyring operation, desktop notification, source repository mutation, or Garnish runtime-state mutation was performed.

## Accepted contract decisions

- Operators manage projects and objectives; Garnish manages internal task/run mechanics.
- The first backlog is user-authored local objectives with deterministic one-task generation in TB-1.
- Existing low-level commands survive beneath `advanced` while replacements gain evidence.
- The calendar is system `W`/`O`/`B` plus project `work`/`non_work`/`both`; `WWWOOBB` is the intended setup default, not a silent migration rewrite.
- Results default to review, explicit apply/discard, durable evidence, and automatic owned worktree cleanup; no Git commit/push/merge/deploy is implied.
- Subscription agents precede explicitly enabled paid API routes.
- Codex/Claude autonomous execution requires structured integration, isolated provider-specific authentication, and attested containers; prompt scraping and writable host credential mounts are forbidden.
- Garnish owns sandbox authority; AoE is an optional supervised execution-plane adapter.
- Initial protected stores, notification channels, quiet-hour behaviour, cleanup periods, and evidence retention are fixed in the contract.

## Next boundary

TB-1 implements only the project-oriented supervisor with fake execution: schema 21 foundations, effective settings explanation, `WWWOOBB`, project/objective lifecycle, service loop, fake quota-driven Codex-to-Claude handoff, review results, and safe automatic cleanup. It must not add live subscription/provider execution, real credential projection, or paid fallback.

