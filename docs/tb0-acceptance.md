# TB-0 acceptance plan

TB-0 freezes the project-supervisor contract before runtime implementation. It performs no provider request, agent task, container launch, keyring write, desktop notification, source-worktree change, or paid API action.

## Acceptance matrix

| ID | Requirement | Machine evidence |
| --- | --- | --- |
| TB0-01 | Authoritative product direction | CLI Bible and ADR 0016 are indexed and distinguish current compatibility from to-be capability. |
| TB0-02 | Exact normal/advanced boundary | Machine contract contains exactly 18 normal families plus the `advanced` gateway; legacy task/API/runtime families are absent from the normal set. |
| TB0-03 | Stable JSON and exits | Contract freezes success/error envelopes, resource-required fields, output modes, and exit codes 0/2–9. |
| TB0-04 | Material preview | Every material command in the contract supports `--dry-run`. |
| TB0-05 | No secret argv | Secret commands expose protected input/file mechanisms and no token/password/API-key argument. |
| TB0-06 | Parser fixtures | At least one valid fixture per normal family and advanced gateway parses; missing operands/options and legacy normal commands fail. |
| TB0-07 | State machines | Project, objective, task/run, calendar, settings/policy, approval, notification, secret, sandbox, cleanup, handoff, and review-result transitions are specified. |
| TB0-08 | Complete current inventory | Every current top-level/subcommand family and source service has an explicit reuse/replace/advanced disposition. |
| TB0-09 | Safe schema plan | Schema-20 migration plan covers W/O/B calendars, project affinity, objectives, settings, approvals/outbox, secrets, sandbox/review/cleanup, inert activation, backup, and rollback. |
| TB0-10 | Design questions closed | Backlog source, command names, auth boundary, structured approvals, keyring fallback, retention, notifications, working windows, integration default, and AoE authority are explicit. |
| TB0-11 | Documentation hygiene | No stale link to the retired unarchived CLI-MVP path, diff whitespace is clean, and command-placeholder policy passes. |
| TB0-12 | Quota-free regression | Contract test, workspace tests, formatting, and strict Clippy pass with provider/live-test variables removed. |

## Command used for TB-0 evidence

Placeholders: none. This command makes no external/provider request, launches no container or agent, touches no keyring/notification service, and consumes no subscription/API quota. Existing deterministic AoE and web UI tests bind temporary local loopback sockets.

```console
./scripts/test-tb0-contract
```

The script explicitly removes provider credentials, live-test acknowledgements, live receipt selectors, and real-container selectors from every Cargo test/lint process. It runs the focused contract parser, placeholder check, formatting, strict Clippy, and complete workspace suite.

TB-0 exits only when these commands pass and the working diff contains documentation/contracts/tests rather than project-supervisor runtime claims. Live Codex, Claude, API, container, notification, and credential tests are neither required nor authorised by this plan.
