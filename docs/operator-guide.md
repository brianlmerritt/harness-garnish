# CLI operator guide

This guide covers the installed narrow CLI MVP. It assumes `garnish --version` succeeds and uses one explicit data directory throughout. Installation and native archive creation are documented in [`installation.md`](installation.md).

## Command and safety conventions

Every copyable block declares its placeholders. A literal acknowledgement beginning `I_ACCEPT_` is not a placeholder and must be typed exactly as shown. Garnish never treats an environment variable, an old quota observation, or an existing API balance as permission to execute provider work.

The examples use a persistent shell variable for Garnish state:

Placeholders: none.

```console
export GARNISH_DATA_DIR="$HOME/.local/share/harness-garnish"
garnish init
garnish doctor
```

Successful commands write JSON to stdout. Readiness and review are machine-readable decisions, not informal advice. A timeout, interrupted provider process, transport uncertainty, or malformed authoritative response must not be retried automatically.

## Register a repository

The repository must already be a Git repository with at least one commit. Garnish records its absolute canonical path and does not copy provider credentials into it.

Placeholders: `/absolute/path/to/repository` must be replaced with the absolute path of the Git repository to register. `PROJECT_SLUG` must be replaced with a short unique slug; the example title may be changed normally.

```console
garnish project add --slug PROJECT_SLUG --title "Example project" --path /absolute/path/to/repository
garnish project list
```

Registered source checkouts should be clean before task execution. Garnish creates isolated worktrees for implementation and detached worktrees for verification; it does not commit, merge, push, or deploy the accepted patch.

The repository must have an existing commit because isolated worktrees require a valid `HEAD`. If worktree preparation fails because `HEAD` is missing or because project files outside `.harness-garnish/` are dirty, the task is recorded as `failed` before an implementer run exists. Fixing or committing the repository does not silently reactivate that durable task. The current MVP has no operator requeue command for this pre-run failure: retain the failed task as evidence and create a replacement task. `garnish recover` is for expired claims and interrupted runs and does not reset a terminal worktree-preparation failure; runtime retry settings apply only after supervised run evidence exists.

## Quota-free first task

Use the deterministic fake adapter first. It exercises project state, quota routing, isolated patch creation, detached verification, and review without contacting a provider.

Placeholders: `PROJECT_SLUG` must be replaced with the registered project slug. `paste-the-id-returned-by-task-add-here` must be replaced with the exact task `id` printed by the first command; keep the surrounding single quotes.

```console
garnish quota set --provider fake --account test --surface five_hour --remaining-percent 90
garnish task add --project PROJECT_SLUG --title "Packaged fixture" --goal "Create result.txt" --accept "result.txt contains done" --verify-argv '["grep","-qx","done","result.txt"]' --scope result.txt --non-scope "all other files; Git integration" --risk-class 1 --fake-write-path result.txt --fake-write-content done
TASK_ID='paste-the-id-returned-by-task-add-here'
garnish task readiness "$TASK_ID" --adapter fake --provider fake --account test
garnish task run "$TASK_ID" --adapter fake --provider fake --account test
garnish task review "$TASK_ID"
```

Before accepting the result, confirm that readiness reported `allowed: true`, the final task status is `review`, `verification.passed` is `true`, `handoff.changed_files` contains only `result.txt`, and `integration_authorized` is `false`. Inspect the exact file at `artifacts.patch_path`. The registered source HEAD and tracked project files remain unchanged until the operator deliberately integrates the reviewed patch. Garnish does create and update its bounded human-readable state beneath `.harness-garnish/`; those files are projections of canonical SQLite state, not integration of the task patch.

For this quota-free fixture, no source integration is required. After inspecting and accepting the review evidence, mark the task complete:

Placeholders: none if `TASK_ID` remains set from the preceding declared assignment.

```console
garnish task complete "$TASK_ID"
garnish task show "$TASK_ID"
```

`task complete` records `user_accepted_review` and promotes any tasks whose dependencies are now satisfied. It does not apply the patch, commit, merge, push, or otherwise authorize integration. For a real implementation task, perform the separately governed manual integration first, then complete the reviewed task when its acceptance state is truthful.

## Codex subscription task

### Host setup

Install and authenticate Codex independently on every host. On Ubuntu and WSL2, install bubblewrap first. These setup commands download software and can modify user authentication state, but do not submit an agent task.

Placeholders: none; these package commands are for Ubuntu or another Debian-family distribution.

```console
sudo apt update
sudo apt install -y bubblewrap curl
curl -fsSL https://chatgpt.com/codex/install.sh | sh
codex --version
codex login --device-auth
codex login status
```

On a graphical system, `codex login` may be used instead of device authentication. Use one login flow. Never copy, commit, or share the Codex authentication file.

Record a fresh supported capability observation. This runs only version/help probes and does not submit a task.

Placeholders: none.

```console
garnish agent refresh --valid-seconds 300
garnish agent status
```

Prefer a current CodexBar quota refresh where available. For a manually observed initial run, record the percentage immediately before task readiness.

Placeholders: `OBSERVED_PERCENT` must be replaced with the current numeric remaining percentage observed immediately before the command. Do not include a percent sign or descriptive text.

```console
garnish quota set --provider codex --account default --surface five_hour --remaining-percent OBSERVED_PERCENT --reserve-percent 20 --source "manual pre-run observation"
```

A manual observation is durable operator evidence, not proof that it remains current later. Replace it before a later task or use the bounded CodexBar collector.

### Create, pin, and inspect the task

Codex subscription execution accepts risk-class 1 tasks with exact safe repository-relative scope and deterministic verification.

Placeholders: `PROJECT_SLUG` must be replaced with the registered project slug. `paste-the-id-returned-by-task-add-here` must be replaced with the exact returned task `id`; keep the surrounding single quotes.

```console
garnish task add --project PROJECT_SLUG --title "Create result" --goal "Create result.txt containing exactly done" --accept "result.txt contains done" --verify-argv '["grep","-qx","done","result.txt"]' --scope result.txt --non-scope "all other files; Git integration" --risk-class 1
TASK_ID='paste-the-id-returned-by-task-add-here'
garnish task pin "$TASK_ID" --adapter codex --provider codex --account default --reason "explicit one-task subscription selection"
garnish task readiness "$TASK_ID" --adapter codex --provider codex --account default
```

Continue only when readiness reports `allowed: true` and the selected identity is exactly `codex:codex:default`.

### Execute exactly one subscription task

The following command can consume Codex subscription quota. It permits exactly one Codex candidate, one active claim, no paid-API fallback, and no automatic retry of an uncertain attempt.

Placeholders: none. `I_ACCEPT_ONE_CODEX_SUBSCRIPTION_TASK` is the required literal acknowledgement.

```console
garnish scheduler daemon --instance codex-local --candidate codex:codex:default --execute-codex --acknowledge-codex I_ACCEPT_ONE_CODEX_SUBSCRIPTION_TASK
```

After success, inspect the review bundle.

Placeholders: none if `TASK_ID` remains set from the preceding declared assignment.

```console
garnish task review "$TASK_ID"
```

Confirm the task is in `review`, verification passed, changed files exactly match task scope, and integration remains unauthorized. If the daemon reports timeout, cancellation, truncation, malformed JSONL, nonzero exit, or patch rejection, stop and inspect the recorded state. Do not immediately resubmit the same work.

## Paid OpenAI or Anthropic API task

Paid API execution is separate from subscription quota and requires four independent pieces: a protected credential reference, project budget, effective model price, and exact current task plan. The acknowledged daemon session enables only its explicitly named API candidates; configuration alone remains inert.

### Protected credential

Prefer an environment variable supplied to the daemon process, a protected Unix file, or macOS Keychain. The database stores only the locator.

Placeholders: `YOUR_PROVIDER_API_KEY` must be replaced with the real provider key in the current private shell. The example variable name is literal for OpenAI; use `ANTHROPIC_API_KEY` and `env:ANTHROPIC_API_KEY` for Anthropic.

```console
export OPENAI_API_KEY='YOUR_PROVIDER_API_KEY'
```

Do not put the value in a Garnish argument, task, plan, repository file, command transcript, or chat. A `file:` secret must be an absolute regular file owned by the Garnish user with no group/other permissions; symlinks are rejected.

### Budget and price evidence

Use the provider's current official price list and an exact model identifier. Garnish does not fetch or guess either. Currency values are integer currency micros per one million tokens. Cache categories must be supplied even when the chosen provider/model publishes a zero or inapplicable rate.

Placeholders: `PROJECT_SLUG`, `PROVIDER`, `ACCOUNT`, `SECRET_REFERENCE`, `EXACT_MODEL_ID`, `PERIOD_START`, and `PERIOD_END` must be replaced with the registered project, literal supported provider (`openai` or `anthropic`), Garnish account label, protected locator, exact model ID, and inclusive/exclusive RFC3339 budget timestamps. `PRICE_SOURCE`, `PRICE_EFFECTIVE_FROM`, and each `*_MICROS_PER_MILLION` value must be replaced with current human-verifiable price evidence expressed as integer currency micros. The example limits and `usd` currency are literals and may be deliberately changed.

```console
garnish api budget-set --project PROJECT_SLUG --provider PROVIDER --account ACCOUNT --enabled true --secret-reference SECRET_REFERENCE --currency usd --currency-limit-micros 500000 --token-limit 100000 --request-limit 10 --period-start PERIOD_START --period-end PERIOD_END --model EXACT_MODEL_ID --tool submit_patch --role implementer --max-output-tokens 512 --max-retries 0 --max-concurrent-requests 1 --reason "explicit bounded project API budget"
garnish api price-set --provider PROVIDER --account ACCOUNT --model EXACT_MODEL_ID --currency usd --input-micros-per-million INPUT_MICROS_PER_MILLION --cached-input-micros-per-million CACHED_INPUT_MICROS_PER_MILLION --cache-creation-input-micros-per-million CACHE_CREATION_INPUT_MICROS_PER_MILLION --output-micros-per-million OUTPUT_MICROS_PER_MILLION --effective-from PRICE_EFFECTIVE_FROM --source PRICE_SOURCE --reason "operator verified current provider price"
garnish api budget-status --project PROJECT_SLUG
garnish api price-status
```

The example budget permits at most ten requests and a total of 0.5 units of `usd` expressed as 500,000 micros during the declared period. Choose limits deliberately for the real account. A zero retry limit is recommended for initial operation.

### Exact task and request plan

An implementation task must be risk class 1, require `agent.patch_submission`, and name every writable path exactly. The plan is bound to the task's current version, so changing the task makes an earlier plan stale.

Placeholders: `PROJECT_SLUG`, `PROVIDER`, `ACCOUNT`, and `EXACT_MODEL_ID` retain their meanings from the preceding block. `paste-the-id-returned-by-task-add-here` must be replaced with the exact returned task `id`; keep the surrounding single quotes.

```console
garnish task add --project PROJECT_SLUG --title "API implementation" --goal "Create result.txt containing exactly done" --accept "result.txt contains done" --verify-argv '["grep","-qx","done","result.txt"]' --scope result.txt --non-scope "all other files; Git integration" --risk-class 1 --requires-capability agent.patch_submission
TASK_ID='paste-the-id-returned-by-task-add-here'
garnish task pin "$TASK_ID" --adapter api --provider PROVIDER --account ACCOUNT --reason "explicit paid API selection"
garnish api plan-set --task "$TASK_ID" --provider PROVIDER --account ACCOUNT --enabled true --model EXACT_MODEL_ID --role implementer --max-input-tokens 4096 --max-output-tokens 512 --max-retries 0 --stream false --reason "exact initial implementation request"
garnish api plan-status --task "$TASK_ID"
garnish task readiness "$TASK_ID" --adapter api --provider PROVIDER --account ACCOUNT
```

Continue only when readiness reports `allowed: true`, the exact API identity is selected, the plan is current, and the displayed reservations fit the intended budget.

### Execute the API patch task

The command can make chargeable requests. Both acknowledgements are required for isolated patch execution. Replace only the declared provider and account components.

Placeholders: `PROVIDER` must be replaced with `openai` or `anthropic`; `ACCOUNT` must be replaced with the exact configured Garnish account label. Both `I_ACCEPT_...` values are required literals.

```console
garnish scheduler daemon --instance paid-patch-local --candidate api:PROVIDER:ACCOUNT --execute-api --acknowledge-paid-api I_ACCEPT_PAID_API_TASK_EXECUTION --execute-api-patches --acknowledge-api-patches I_ACCEPT_ISOLATED_API_PATCH_EXECUTION
```

After success, inspect review and accounting.

Placeholders: none if `TASK_ID` remains set from the preceding declared assignment.

```console
garnish task review "$TASK_ID"
garnish api reservations
garnish api attempts
garnish api spend
```

Confirm verification and exact changed-file scope before any manual integration. Authentication or other terminal provider errors fail immediately. Transport uncertainty, malformed authoritative output, or interruption retains an uncertain dispatch and cannot replay automatically. Do not treat absence of a passing review as permission to retry.

## Read-only dashboard

The dashboard exposes canonical state but cannot mutate it.

Placeholders: none.

```console
garnish ui serve --port 7467
```

Open the exact one-time loopback URL printed by Garnish. The token is sensitive while the process is running. The listener binds to `127.0.0.1`; for a remote machine, create an operator-controlled SSH tunnel rather than changing the bind address. Stop the dashboard with `Ctrl-C`.

## Operations, backup, and recovery

Inspect state and create a private integrity-checked backup regularly and before upgrades.

Placeholders: none.

```console
garnish ops status
garnish ops diagnostics
garnish ops backup
garnish notification list
```

Pause prevents new claims. Emergency stop also requests cancellation and releases work that has not started.

Placeholders: none; the reasons are literal examples and may be replaced with truthful operator reasons.

```console
garnish ops pause --reason "operator maintenance"
garnish ops status
garnish ops resume --reason "maintenance complete"
garnish ops emergency-stop --reason "operator emergency stop"
```

After an unclean process exit, run recovery once and inspect the resulting state. Recovery expires or requeues only transitions that the durable evidence proves safe; it does not replay uncertain provider work.

Placeholders: none.

```console
garnish recover
garnish ops status
garnish notification list
```

Inspect runs for an affected task separately.

Placeholders: `TASK_ID` must be replaced with the exact affected task ID.

```console
garnish runtime runs --task TASK_ID
```

## Current boundaries

- The dashboard is read-only; mutation remains CLI-only.
- MCP configuration is registration evidence only. Garnish cannot launch an MCP server or call a tool.
- Claude Code, Antigravity, AoE, and arbitrary CLI task execution are not part of this MVP.
- Garnish prepares and verifies patches but does not commit, merge, push, deploy, or authorize integration.
- The command verifier is deterministic command execution in a detached worktree, not semantic agent review.
- There is no automatic provider fallback, automatic update, packaged service manager, or production-readiness claim.
