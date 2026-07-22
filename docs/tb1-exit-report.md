# TB-1 project-supervisor exit report

Date: 2026-07-21  
Branch observed: `pivot_cli`  
Evidence command: `./scripts/test-tb1-cli`

## Outcome

TB-1 exited successfully on macOS subject to the user's normal commit process. The executable now offers a project-first, quota-free vertical slice backed by SQLite schema 21 and deterministic fake Codex/Claude profiles. An operator can register a project, add an objective, start supervision, let the service select an eligible project and subscription profile, inspect a verified result, and apply or discard it without creating or managing a task ID, worktree, or branch.

This is real control-plane and Git evidence over fake agent execution. It is not evidence of live Codex, Claude Code, API, container, credential, approval, notification, or web execution.

## Implemented boundary

- transactional schema-20-to-21 migration with verified backup, foreign keys, inert legacy-project activation, and clean-install schema equivalence;
- revisioned `W`/`O`/`B` calendars, `WWWOOBB` init default, shared-day exceptions, seven-day previews, and project `work`/`non_work`/`both` affinity;
- project registration plus start, pause, resume, stop, status, archive, calendar, and affinity controls;
- local objectives with exactly one deterministic internal implementation task;
- a continuous foreground `service run` loop and bounded `--max-cycles` form for diagnostics/tests;
- deterministic multi-project eligible-day selection;
- fake Codex preferred routing, fake Claude handoff when fake Codex lacks headroom, and no paid fallback;
- isolated implementation worktree, separate verifier, durable review result, explicit apply/discard, and exact source-base check;
- automatic cleanup of exactly owned implementation/verifier worktrees and task branches, with quarantine evidence on ambiguity;
- revisioned global setting explanation for the TB-1 subset;
- hidden legacy parser aliases plus the canonical `advanced` compatibility gateway;
- detailed, placeholder-declared operator exercises in [TB-1 CLI testing](tb1-cli-testing.md).

## Machine evidence

The final wrapper removed provider credentials, live acknowledgements, container image opt-ins, and persistent data-directory overrides before execution. It reported:

- command-placeholder declaration check: passed;
- `cargo fmt --all -- --check`: passed;
- 4 project-supervisor CLI integration tests: passed, including terminal cleanup after a deliberately failed attempt;
- schema-21 migration test: passed;
- strict workspace/all-target/all-feature Clippy: passed;
- full workspace regression: 171 passed, 0 failed, 5 ignored;
- ignored tests: two real container backends and three separately acknowledged live Codex/API smokes;
- external network/provider/container/keyring/notification use: none;
- local loopback use: only the established deterministic AoE and web UI regressions.

The integration evidence includes a real temporary Git repository. Before apply, the requested file is absent and only the source worktree remains. After apply, `tb1-result.txt` contains exactly `done`; source `HEAD` remains unchanged because Git integration is not authorised.

One pre-existing process-output fixture exposed a scheduling race during the first full run. The fixture now signals that bounded output exists before requesting cancellation, preserving the original process contract without relying on a sleep. The final full run passed.

## Deliberate residual scope

- Output remains transitional JSON-only; human formatting and the full granular exit-code contract remain unactivated.
- Only the TB-1 settings subset is effective. Complete layered settings, approvals, notifications, and protected secret storage are TB-2.
- Agent profiles are fixed fake fixtures. Real attested-container Codex is TB-3 and Claude Code/switching is TB-4.
- `--fixture-write-path` and `--fixture-write-content` exist solely to make quota-free patch review observable. They do not define the future real-agent interface.
- Compatibility aliases remain hidden but accepted; `advanced` is the documented canonical route.
- The service is foreground-only. OS service installation, web parity, signed distribution, and cross-platform release acceptance remain TB-6.
- This local exit does not replace later Linux/WSL2 portability evidence for a release candidate.

The next product phase is TB-2: complete layered settings, structured approvals, notification outbox/delivery, and secure secret metadata/backends before any real agent receives projected authority.
