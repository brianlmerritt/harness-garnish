# Test the TB-1 CLI yourself

These exercises use the actual compiled CLI, a real temporary Git repository, persistent SQLite state, real worktrees and patches, and the fake Codex/Claude execution profiles. They consume no subscription or API quota and do not use your existing projects.

## 1. Build and create an isolated test project

Placeholders: none. All paths below are created and assigned by the commands themselves.

```console
cargo build --locked
TB1_GARNISH='./target/debug/garnish'
TB1_STATE="$(mktemp -d -t garnish-tb1-state.XXXXXX)"
TB1_REPO="$(mktemp -d -t garnish-tb1-repo.XXXXXX)"
git -C "$TB1_REPO" init -q --initial-branch=main
git -C "$TB1_REPO" config user.email fixture@example.invalid
git -C "$TB1_REPO" config user.name Fixture
git -C "$TB1_REPO" config commit.gpgsign false
printf 'TB-1 fixture\n' > "$TB1_REPO/README.md"
git -C "$TB1_REPO" add README.md
git -C "$TB1_REPO" commit -q -m fixture
printf 'state=%s\nrepository=%s\n' "$TB1_STATE" "$TB1_REPO"
```

Keep the printed paths until the exercise is finished. No change is made to the Harness Garnish repository by the following workflow.

## 2. Initialise and inspect the calendar/settings

Placeholders: none. This block relies on the three shell variables created in step 1.

```console
"$TB1_GARNISH" --data-dir "$TB1_STATE" init --calendar-pattern WWWOOBB --timezone Europe/London
"$TB1_GARNISH" --data-dir "$TB1_STATE" config explain calendar.default.pattern
"$TB1_GARNISH" --data-dir "$TB1_STATE" calendar preview default --from 2026-07-20 --days 7
```

The preview should show `W`, `W`, `W`, `O`, `O`, `B`, `B` in Monday-to-Sunday order.

## 3. Register a project and create an objective

The two `--fixture-write-*` options are explicitly TB-1 fake-backend controls. They make the fake agent produce a real reviewed patch so result application is observable; they are not a claim about the future real-agent interface.

Placeholders: none. This block relies on the three shell variables created in step 1.

```console
"$TB1_GARNISH" --data-dir "$TB1_STATE" project add "$TB1_REPO" --slug tb1-demo --title 'TB-1 demo' --affinity both
"$TB1_GARNISH" --data-dir "$TB1_STATE" objective add tb1-demo --title 'Create a verified result' --goal 'Create tb1-result.txt containing done' --accept 'tb1-result.txt contains done' --fixture-write-path tb1-result.txt --fixture-write-content done
"$TB1_GARNISH" --data-dir "$TB1_STATE" project start tb1-demo
"$TB1_GARNISH" --data-dir "$TB1_STATE" project status tb1-demo
```

The objective is visible as `ready`; no task ID is needed.

## 4. Force the quota handoff and run one service cycle

This changes only the fake `codex-subscription` quota in the temporary state. It does not inspect or consume your real Codex allowance.

Placeholders: none. This block relies on the three shell variables created in step 1.

```console
"$TB1_GARNISH" --data-dir "$TB1_STATE" agent configure codex-subscription quota.remaining_percent 10 --reason 'exercise quota handoff'
"$TB1_GARNISH" --data-dir "$TB1_STATE" route explain tb1-demo
"$TB1_GARNISH" --data-dir "$TB1_STATE" service run --max-cycles 1
"$TB1_GARNISH" --data-dir "$TB1_STATE" project status tb1-demo
"$TB1_GARNISH" --data-dir "$TB1_STATE" maintenance preview --project tb1-demo
git -C "$TB1_REPO" worktree list
git -C "$TB1_REPO" branch --list 'garnish/task-*'
```

The cycle should report fake Claude selected after fake Codex is declined. Project status should show a pending review and `cleanup_status: complete`. The source checkout should still lack `tb1-result.txt`; the worktree list should contain only the source checkout and the branch command should print nothing.

## 5. Review and apply without manually handling a worktree

The first command derives the result ID from Garnish output; there is no placeholder to paste.

Placeholders: none. This block relies on the three shell variables created in step 1.

```console
TB1_RESULT_ID="$("$TB1_GARNISH" --data-dir "$TB1_STATE" project review tb1-demo | sed -n 's/^[[:space:]]*"id": "\([^"]*\)",/\1/p' | head -n 1)"
printf 'result=%s\n' "$TB1_RESULT_ID"
"$TB1_GARNISH" --data-dir "$TB1_STATE" project apply "$TB1_RESULT_ID" --reason 'accepted TB-1 fixture result'
test "$(cat "$TB1_REPO/tb1-result.txt")" = done
"$TB1_GARNISH" --data-dir "$TB1_STATE" project stop tb1-demo --reason 'exercise complete'
"$TB1_GARNISH" --data-dir "$TB1_STATE" project status tb1-demo
```

The objective should now be `completed`, the project should be `stopped`, and `tb1-result.txt` should contain exactly `done`. The file is deliberately uncommitted: Garnish does not claim Git integration authority.

## 6. Other safe lifecycle checks

Placeholders: none. This block relies on the three shell variables created in step 1.

```console
"$TB1_GARNISH" --data-dir "$TB1_STATE" project start tb1-demo
"$TB1_GARNISH" --data-dir "$TB1_STATE" project pause tb1-demo --reason 'operator pause test'
"$TB1_GARNISH" --data-dir "$TB1_STATE" project resume tb1-demo
"$TB1_GARNISH" --data-dir "$TB1_STATE" status --project tb1-demo
"$TB1_GARNISH" --data-dir "$TB1_STATE" advanced task list --project tb1-demo
```

The last command demonstrates the compatibility gateway. Normal project operation does not require it.

The temporary repository and state are left in place so you can inspect them. Their exact paths were printed in step 1; remove those two directories when you decide you no longer need the evidence.
