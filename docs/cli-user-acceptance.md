# Harness Garnish CLI user acceptance

Status: living, hands-on user acceptance guide for the project-supervisor CLI. This is the human counterpart to the engineering [CLI acceptance plan](cli-acceptance.md). It shows what a user can actually try at the end of each `TB` phase, what successful behaviour looks like, and what remains outside that phase.

## How this document is maintained

At the end of each implementation phase, add a self-contained section that starts from fresh temporary Garnish state and disposable repositories. The newest section must exercise the complete user-visible journey available at that phase, not merely the commands added during the phase. Older sections remain dated phase snapshots.

These walkthroughs do not replace automated phase acceptance. They let a user judge whether Garnish is becoming the product described in the [CLI Bible](cli-bible.md): projects are the normal unit, status is useful, and Garnish handles internal task, route, worktree, verification, handoff, and cleanup mechanics.

## TB-1 — Project-oriented supervisor with fake execution

TB-1 is the first usable project-first slice. The commands below use real CLI parsing, SQLite state, Git repositories, worktrees, patches, detached verification, and cleanup. The Codex and Claude profiles are deterministic fakes: this walkthrough consumes no subscription quota or API budget and does not use credentials, containers, keyrings, notifications, or existing projects.

All output is transitional JSON in TB-1. IDs, timestamps, and temporary paths vary between runs; inspect the named fields rather than comparing the complete output byte for byte.

### 1. Confirm the visible command surface

Placeholders: none. `GARNISH_BIN` may already name an executable in the shell; otherwise this block uses `garnish` from `PATH`.

```console
GARNISH_BIN="${GARNISH_BIN:-garnish}"
"$GARNISH_BIN" --version
"$GARNISH_BIN" --help
```

The help should centre on `status`, `service`, `config`, `calendar`, `project`, `objective`, `agent`, `route`, and `maintenance`. Low-level `task`, `scheduler`, `runtime`, `api`, and `mcp` families should not be advertised as normal commands. `advanced` should be available for compatibility diagnostics.

This does not mean every visible family is complete. The exact TB-1 boundary is listed at the end of this section.

### 2. Create isolated state and two disposable projects

Placeholders: none. This block creates and assigns all paths itself.

```console
TB1_STATE="$(mktemp -d -t garnish-tb1-user-state.XXXXXX)"
TB1_WORK_REPO="$(mktemp -d -t garnish-tb1-user-work.XXXXXX)"
TB1_NONWORK_REPO="$(mktemp -d -t garnish-tb1-user-nonwork.XXXXXX)"
for TB1_REPO in "$TB1_WORK_REPO" "$TB1_NONWORK_REPO"; do
  git -C "$TB1_REPO" init -q --initial-branch=main
  git -C "$TB1_REPO" config user.email fixture@example.invalid
  git -C "$TB1_REPO" config user.name Fixture
  git -C "$TB1_REPO" config commit.gpgsign false
  git -C "$TB1_REPO" config core.hooksPath /dev/null
  printf 'TB-1 user acceptance fixture\n' > "$TB1_REPO/README.md"
  git -C "$TB1_REPO" add README.md
  git -C "$TB1_REPO" commit -q -m fixture
done
TB1_WORK_HEAD="$(git -C "$TB1_WORK_REPO" rev-parse HEAD)"
TB1_NONWORK_HEAD="$(git -C "$TB1_NONWORK_REPO" rev-parse HEAD)"
printf 'state=%s\nwork_project=%s\nnonwork_project=%s\n' "$TB1_STATE" "$TB1_WORK_REPO" "$TB1_NONWORK_REPO"
```

Keep the printed paths for inspection. Every following Garnish command uses `TB1_STATE`, so it does not open the user's normal Garnish database.

### 3. Initialise Garnish and inspect the calendar

Placeholders: none. This block relies on the variables created in steps 1 and 2.

```console
"$GARNISH_BIN" --data-dir "$TB1_STATE" init --calendar-pattern WWWOOBB --timezone Europe/London
"$GARNISH_BIN" --data-dir "$TB1_STATE" config explain calendar.default.pattern
"$GARNISH_BIN" --data-dir "$TB1_STATE" calendar preview default --from 2026-07-20 --days 7
```

Look for:

- `weekly_pattern` or `effective_value` equal to `WWWOOBB`;
- timezone `Europe/London`;
- Monday through Sunday classified as `W`, `W`, `W`, `O`, `O`, `B`, `B`;
- an explanation showing where the effective calendar setting came from.

### 4. Register one work project and one non-work project

Placeholders: none. This block relies on the variables created in steps 1 and 2.

```console
"$GARNISH_BIN" --data-dir "$TB1_STATE" project add "$TB1_WORK_REPO" --slug tb1-work --title 'TB-1 work project' --affinity work
"$GARNISH_BIN" --data-dir "$TB1_STATE" project add "$TB1_NONWORK_REPO" --slug tb1-nonwork --title 'TB-1 non-work project' --affinity non-work
"$GARNISH_BIN" --data-dir "$TB1_STATE" project list
"$GARNISH_BIN" --data-dir "$TB1_STATE" project show tb1-work
"$GARNISH_BIN" --data-dir "$TB1_STATE" project show tb1-nonwork
```

Both projects should be registered in `stopped` state. The first should have `work` affinity and the second `non_work` affinity. No internal task ID, route pin, scheduler instance, or worktree command is required.

### 5. See how the same calendar applies to each project

Placeholders: none. This block relies on the variables created in steps 1 and 2.

```console
"$GARNISH_BIN" --data-dir "$TB1_STATE" calendar preview default --project tb1-work --from 2026-07-20 --days 7
"$GARNISH_BIN" --data-dir "$TB1_STATE" calendar preview default --project tb1-nonwork --from 2026-07-20 --days 7
```

For the work project, Monday to Wednesday and the shared weekend should be eligible; Thursday and Friday should be ineligible. For the non-work project, Thursday, Friday, and the shared weekend should be eligible; Monday to Wednesday should be ineligible.

### 6. Give each project a user-level objective and start supervision

The `--fixture-write-*` options are TB-1 fake-backend controls. They make reviewed patches observable and are not the future real-agent interface.

Placeholders: none. This block relies on the variables created in steps 1 and 2.

```console
"$GARNISH_BIN" --data-dir "$TB1_STATE" objective add tb1-work --title 'Create the work result' --goal 'Create tb1-work-result.txt containing done' --accept 'tb1-work-result.txt contains done' --fixture-write-path tb1-work-result.txt --fixture-write-content done
"$GARNISH_BIN" --data-dir "$TB1_STATE" objective add tb1-nonwork --title 'Create the non-work result' --goal 'Create tb1-nonwork-result.txt containing done' --accept 'tb1-nonwork-result.txt contains done' --fixture-write-path tb1-nonwork-result.txt --fixture-write-content done
"$GARNISH_BIN" --data-dir "$TB1_STATE" project start tb1-work
"$GARNISH_BIN" --data-dir "$TB1_STATE" project start tb1-nonwork
"$GARNISH_BIN" --data-dir "$TB1_STATE" status
```

Both projects should be active and their objectives ready. Normal status should show projects and objectives, not require the user to manage the internal tasks Garnish created.

### 7. Exercise automatic calendar selection and fake quota handoff

The quota command below changes only the immutable fake `codex-subscription` profile in `TB1_STATE`. The two `--at` values are bounded TB-1 fixture controls: 23 July 2026 is an `O` day and 20 July 2026 is a `W` day in the chosen calendar.

Placeholders: none. This block relies on the variables created in steps 1 and 2.

```console
"$GARNISH_BIN" --data-dir "$TB1_STATE" agent configure codex-subscription quota.remaining_percent 10 --reason 'user acceptance handoff'
"$GARNISH_BIN" --data-dir "$TB1_STATE" route explain tb1-nonwork
"$GARNISH_BIN" --data-dir "$TB1_STATE" service run --max-cycles 1 --at 2026-07-23T12:00:00Z
"$GARNISH_BIN" --data-dir "$TB1_STATE" route explain tb1-work
"$GARNISH_BIN" --data-dir "$TB1_STATE" service run --max-cycles 1 --at 2026-07-20T12:00:00Z
"$GARNISH_BIN" --data-dir "$TB1_STATE" status
```

Look for:

- the `O`-day cycle selecting `tb1-nonwork` and the `W`-day cycle selecting `tb1-work`;
- fake Codex declined for insufficient quota;
- fake Claude selected without a paid API fallback;
- both objectives moving to review;
- route and cycle output explaining the selection rather than asking for a manual pin.

### 8. Inspect review evidence and automatic cleanup

Placeholders: none. This block relies on the variables created in steps 1 and 2.

```console
"$GARNISH_BIN" --data-dir "$TB1_STATE" project status tb1-work
"$GARNISH_BIN" --data-dir "$TB1_STATE" project status tb1-nonwork
"$GARNISH_BIN" --data-dir "$TB1_STATE" project review tb1-work
"$GARNISH_BIN" --data-dir "$TB1_STATE" project review tb1-nonwork
"$GARNISH_BIN" --data-dir "$TB1_STATE" maintenance preview --project tb1-work
git -C "$TB1_WORK_REPO" worktree list
git -C "$TB1_NONWORK_REPO" worktree list
git -C "$TB1_WORK_REPO" branch --list 'garnish/task-*'
git -C "$TB1_NONWORK_REPO" branch --list 'garnish/task-*'
test ! -e "$TB1_WORK_REPO/tb1-work-result.txt"
test ! -e "$TB1_NONWORK_REPO/tb1-nonwork-result.txt"
```

Each status should show a pending review and `cleanup_status` equal to `complete`. Each worktree list should contain only its source checkout, both branch commands should print nothing, and neither result file should yet be present in the source checkout.

This is the important TB-1 boundary: Garnish created and verified work, captured review evidence, and removed its owned execution resources without making the user manage them.

### 9. Apply one result and discard the other

The first two commands derive the result IDs from Garnish output; there are no IDs to copy and paste manually.

Placeholders: none. This block relies on the variables created in steps 1 and 2.

```console
TB1_WORK_RESULT_ID="$("$GARNISH_BIN" --data-dir "$TB1_STATE" project review tb1-work | sed -n 's/^[[:space:]]*"id": "\([^"]*\)",/\1/p' | head -n 1)"
TB1_NONWORK_RESULT_ID="$("$GARNISH_BIN" --data-dir "$TB1_STATE" project review tb1-nonwork | sed -n 's/^[[:space:]]*"id": "\([^"]*\)",/\1/p' | head -n 1)"
printf 'work_result=%s\nnonwork_result=%s\n' "$TB1_WORK_RESULT_ID" "$TB1_NONWORK_RESULT_ID"
"$GARNISH_BIN" --data-dir "$TB1_STATE" project apply "$TB1_WORK_RESULT_ID" --reason 'accepted user acceptance result'
"$GARNISH_BIN" --data-dir "$TB1_STATE" project discard "$TB1_NONWORK_RESULT_ID" --reason 'exercise discard path'
test "$(cat "$TB1_WORK_REPO/tb1-work-result.txt")" = done
test ! -e "$TB1_NONWORK_REPO/tb1-nonwork-result.txt"
test "$(git -C "$TB1_WORK_REPO" rev-parse HEAD)" = "$TB1_WORK_HEAD"
test "$(git -C "$TB1_NONWORK_REPO" rev-parse HEAD)" = "$TB1_NONWORK_HEAD"
git -C "$TB1_WORK_REPO" status --short
git -C "$TB1_NONWORK_REPO" status --short
```

The applied file should contain exactly `done`; the discarded file should remain absent. Both source `HEAD` values must be unchanged. The work repository should show the applied file as an uncommitted user-review change because Garnish does not have commit, push, merge, PR, or deployment authority.

### 10. Exercise lifecycle controls and the advanced boundary

Placeholders: none. This block relies on the variables created in steps 1 and 2.

```console
"$GARNISH_BIN" --data-dir "$TB1_STATE" project pause tb1-work --reason 'user acceptance pause'
"$GARNISH_BIN" --data-dir "$TB1_STATE" project resume tb1-work
"$GARNISH_BIN" --data-dir "$TB1_STATE" project stop tb1-work --reason 'user acceptance complete'
"$GARNISH_BIN" --data-dir "$TB1_STATE" project stop tb1-nonwork --reason 'user acceptance complete'
"$GARNISH_BIN" --data-dir "$TB1_STATE" status
"$GARNISH_BIN" --data-dir "$TB1_STATE" advanced task list --project tb1-work
```

The final normal status should show both projects stopped. The last command demonstrates that internal tasks remain inspectable for diagnostics, while none of the normal journey required their IDs or direct operation.

The temporary state and repositories remain at the paths printed in step 2 so the user can inspect them and remove them when no longer wanted.

### TB-1 passes this user walkthrough when

- projects and objectives, rather than tasks, drive the normal workflow;
- work and non-work projects are eligible on the correct `WWWOOBB` days;
- Garnish selects the eligible project without a manual claim or scheduler instance;
- low fake Codex quota selects fake Claude and never implies paid API consent;
- verified results remain outside source checkouts until apply;
- apply and discard do not change `HEAD` or perform remote Git effects;
- Garnish removes its owned worktrees and branches automatically;
- status explains enough to follow objectives, review state, next action, and cleanup;
- pause, resume, stop, and advanced diagnostics behave as described.

### What is working at TB-1

- project registration and project-level lifecycle;
- local user objectives with deterministic internal task creation;
- `W`/`O`/`B` calendars, project affinity, exceptions, and previews;
- foreground multi-project scheduling with a bounded diagnostic cycle;
- fake subscription routing and fake Codex-to-Claude quota handoff;
- real isolated Git worktrees, detached verification, review results, apply/discard, and exact owned cleanup;
- narrow settings explanation and project-oriented status;
- `advanced` as the documented compatibility gateway.

### What is still to do after TB-1

- TB-1 uses fake Codex and Claude profiles; it does not run either real subscription agent.
- Output is transitional JSON-only. Human output, versioned JSON envelopes, granular exit codes, global output controls, and complete dry-run behaviour remain to be activated.
- Status is useful for the fixture journey but does not yet answer every quota, approval, notification, container, budget, failure, and next-wake question in the CLI Bible.
- Complete layered settings, structured approvals, transactional notifications, and protected secret management are TB-2.
- Real attested-container Codex execution and credential projection are TB-3.
- Real Claude execution and cross-subscription checkpoint switching are TB-4.
- Explicitly enabled and budgeted paid API fallback is TB-5; low subscription quota alone will never authorise it.
- Installed background services, complete web parity, packaging, upgrades, backup/restore, soak/recovery, and release acceptance are TB-6.
- Legacy top-level aliases are hidden but still temporarily accepted for compatibility.

For additional TB-1 implementation detail and automated evidence, see the [TB-1 acceptance plan](tb1-acceptance.md) and [TB-1 exit report](tb1-exit-report.md).
