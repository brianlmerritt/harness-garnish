# TB-1 project-supervisor acceptance

TB-1 is the first executable project-first slice. It uses real CLI parsing, SQLite persistence, calendar evaluation, Git worktrees, detached verification, review artifacts, result application, and owned-resource cleanup. Codex and Claude are deterministic fake agent profiles in this phase; no provider, API, container, credential store, or notification backend is contacted.

## Required evidence

| ID | Requirement | Machine evidence |
| --- | --- | --- |
| TB1-01 | Schema 20 upgrades transactionally to schema 21 with a verified backup | `db::tests::schema_twenty_one_migration_is_inert_backed_up_and_clean_install_equivalent` |
| TB1-02 | Upgraded projects are inert | The same migration test proves unpaused legacy projects become `stopped` and paused projects remain `paused`. |
| TB1-03 | `WWWOOBB`, `work`, `non_work`, `both`, and `B` exceptions are evaluated correctly | Schedule unit tests and `calendar_preview_applies_work_non_work_and_shared_days` |
| TB1-04 | The operator registers and starts projects and adds objectives without task IDs | `project_first_fixture_routes_hands_off_reviews_and_cleans_without_task_commands` |
| TB1-05 | One deterministic internal task is generated per local objective | The project-first integration test inspects the task only through `advanced` after completing the normal workflow. |
| TB1-06 | Multiple projects are selected only on eligible days | `bounded_service_cycle_selects_the_eligible_project_without_task_ids` |
| TB1-07 | Low fake Codex quota hands work to fake Claude without paid fallback | The project-first integration test records `previous_agent=codex-subscription`, `selected_agent=claude-subscription`, and a quota-insufficient reason. |
| TB1-08 | A verified result remains outside the source checkout until explicit apply | The project-first integration test checks file absence before `project apply` and exact content afterwards. |
| TB1-09 | Owned implementation/verifier worktrees and the Garnish branch are removed automatically | The project-first integration test proves one source worktree remains, no Garnish task branch remains, and cleanup evidence is `complete`. |
| TB1-10 | Failed attempts do not litter the repository | `failed_fixture_attempt_is_blocked_and_its_owned_git_resources_are_removed` deliberately rejects an escaping write, blocks the objective, and proves exact owned cleanup. |
| TB1-11 | Compatibility internals have a canonical `advanced` route | The integration test executes `advanced task list`; legacy top-level aliases remain hidden parser compatibility during the transition. |
| TB1-12 | No live or secret-bearing environment can leak into acceptance | `scripts/test-tb1-cli` removes provider credentials and all live acknowledgements before tests. |

## Acceptance command

Placeholders: none.

```console
./scripts/test-tb1-cli
```

The full regression portion binds only deterministic loopback fixtures already used by the historical AoE and web UI tests. It makes no external connection. Five real-runtime/provider smoke tests remain ignored unless separately and explicitly enabled.

## Honest phase boundary

TB-1 does not execute a real Codex or Claude task. It does not project credentials, start a hardened container, broker agent permission requests, deliver notifications, store secrets, fall back to a paid API, install a background OS service, or provide web parity. Those remain TB-2 through TB-6 work.

Normal output is still JSON in this transitional build; the frozen human/JSON formatting and granular exit-code contract is not yet fully activated. Compatibility command aliases are hidden from normal help but not yet rejected, because existing scripts remain supported through the transition.
