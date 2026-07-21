# Harness Garnish

Harness Garnish is a local control plane for quota-aware, policy-controlled AI-assisted software development. It coordinates existing coding-agent CLIs and optional API-backed agents, while keeping scheduling, approvals, canonical state, verification, and recovery outside the agents themselves.

Phase 0 established the architecture under [`docs/`](docs/README.md). The Phase 1 Rust vertical slice is now implemented and locally verified; its exact evidence, limitations, and deferred smoke tests are recorded in [`docs/phase-1-exit-report.md`](docs/phase-1-exit-report.md).

Phase 2 is complete. Its schema-7 durable scheduler, supervision/recovery matrices, day-aware scheduling, container evidence, and final macOS/Linux/WSL2 results are recorded in [`docs/phase-2-exit-report.md`](docs/phase-2-exit-report.md).

Phase 3 is complete. Its schema-14 multi-agent capability routing, quota observations/reservations/forecasting, authenticated local operator interface, separate verifier runs, and final macOS/Linux/WSL2 results are recorded in [`docs/phase-3-exit-report.md`](docs/phase-3-exit-report.md).

Phase 4 is in progress. Its network-free first slice adds project-scoped API budgets and atomic paid-usage reservations before any real OpenAI or Anthropic request is enabled; see [`docs/phase-4-plan.md`](docs/phase-4-plan.md).

The narrower source-run CLI MVP is complete. Its implemented boundary, cross-platform fixture results, separately opted-in live evidence, and explicit deferrals are recorded in [`docs/cli-mvp-exit-report.md`](docs/cli-mvp-exit-report.md). This does not change the broader Phase 4 status.

## Confirmed direction

- `garnish` will be a small Rust control plane with SQLite as canonical state.
- Agent of Empires (AoE) is the first execution-plane integration, behind a replaceable adapter.
- Docker, rootless Podman, and Apple Container are independent sandbox backends selected by capability probes and policy.
- Codex CLI, Claude Code, and Antigravity CLI are the first agent adapters.
- OpenAI and Anthropic APIs are opt-in per project, with hard project budgets.
- Subscription limits are represented as multiple independently configurable quota surfaces, never as one invented total.
- The default checkpoint interval is five minutes and may be shortened by the quota guard.
- Git mutation policy is project-specific. This repository's branches and commits remain user-managed.

Harness Garnish is not an API proxy, a provider-limit bypass, an autonomous merge/deploy service, or a claim that proprietary conversation state is portable between agents.

## Phase 0 documents

- [Discovery and decisions](docs/phase-0-discovery.md)
- [Architecture](docs/architecture.md)
- [Threat model](docs/threat-model.md)
- [Data model](docs/data-model.md)
- [Policy model](docs/policy-model.md)
- [Adapter contracts](docs/adapter-contracts.md)
- [MVP acceptance tests](docs/mvp-acceptance.md)
- [Architecture decision records](docs/decisions/README.md)

## Install and operate the CLI MVP

The source-run CLI MVP can now be installed through Cargo or packaged as a versioned native archive. The complete supported-platform, checksum, state, upgrade, and uninstall contract is in the [installation and binary-packaging guide](docs/installation.md). The [CLI operator guide](docs/operator-guide.md) provides complete quota-free fixture, Codex subscription, paid API, dashboard, backup, and recovery workflows using the installed `garnish` command.

Create and exercise the current native archive before distributing it. These commands make no provider request, consume no subscription or paid API quota, and publish nothing:

Placeholders: none.

```console
./scripts/package-release
./scripts/test-release-package
```

The accepted initial package targets are Apple Silicon macOS and x86-64 GNU/Linux, including x86-64 WSL2. Archives are checksummed but not signed or notarized. ADR 0015 records the release boundary.

## Phase 1 quick start

Rust 1.97 or newer and Git are required. All normal tests use fake agents and consume no provider quota or API budget.

### Command notation

Every copyable command block is preceded by a `Placeholders:` declaration. `Placeholders: none.` means the block can be copied literally from the repository root. Any other declaration names each value that must be replaced before execution and says where its real value comes from. Uppercase acknowledgement phrases such as `I_ACCEPT_ONE_CODEX_SUBSCRIPTION_TASK` are declared literals, not placeholders. Never paste an undeclared descriptive token into a command.

Placeholders: none.

```console
cargo build --locked
cargo test --workspace
cargo run -- --data-dir /tmp/garnish-state doctor
```

The CLI emits JSON on stdout for success and JSON on stderr for failure. Exit code `0` means the command completed; exit code `1` means validation, policy, quota, adapter, runtime, or state handling rejected or failed the command. Argument syntax errors are emitted by Clap with exit code `2`.

Typical local flow:

Placeholders: `/absolute/path/to/repository` must be replaced with the absolute path of the Git repository Garnish will register. All other values are literal examples.

```console
garnish --data-dir .garnish-state init
garnish --data-dir .garnish-state project add --slug example --title Example --path /absolute/path/to/repository
garnish --data-dir .garnish-state quota set --provider fake --account test --surface five_hour --remaining-percent 90
garnish --data-dir .garnish-state task add --project example --title "Bounded change" --goal "Create result.txt" --accept "result.txt contains done" --verify-argv '["grep","-q","done","result.txt"]' --scope result.txt --non-scope "remote Git" --fake-write-path result.txt --fake-write-content done
```

`task run` is deliberately limited to deterministic fake adapters. Codex subscription execution and paid API execution use separate, explicitly acknowledged scheduler paths described below. Claude Code, Antigravity, AoE, and all real external smoke tests remain disabled in normal tests and default CI.

### Day-aware scheduling

Weekly patterns are Monday through Sunday. `W` means a user workday and `O` an off day; the default is `WWWWWOO`. Tasks use `--day-affinity W`, `O`, or `B` (both, the default).

Placeholders: `TASK_ID` must be replaced with the exact task `id` returned by `task add`.

```console
garnish --data-dir .garnish-state schedule configure --slug uk-week --timezone Europe/London --weekly-pattern WWWWWOO
garnish --data-dir .garnish-state schedule assign --project example --calendar uk-week
garnish --data-dir .garnish-state schedule exception --calendar uk-week --date 2026-12-25 --kind O --reason "holiday"
garnish --data-dir .garnish-state schedule evaluate --task TASK_ID
garnish --data-dir .garnish-state schedule preview --provider fake --account test
```

Calendar gating is external to the agents. An ineligible task remains ready with an explained next wake time; a running task that crosses a day boundary requests a checkpoint and graceful pause at the next safe boundary.

The scheduler arbitration commands are deliberately explicit while the daemon is being built:

Placeholders: none; `local-1`, `my-mac`, `fake`, and `test` are literal example identities that may be kept for a local fixture flow.

```console
garnish --data-dir .garnish-state scheduler register --instance local-1 --hostname my-mac
garnish --data-dir .garnish-state scheduler acquire-leader --instance local-1
garnish --data-dir .garnish-state scheduler tick --instance local-1 --generation 1 --provider fake --account test --max-active 2 --max-active-per-adapter 1 --max-active-per-account 1
garnish --data-dir .garnish-state scheduler wakes
```

Leadership is fenced by a monotonically increasing generation. Claims, global/adapter/account concurrency checks, and project locks commit atomically; a stale leader cannot claim work. Scheduler exclusions are persisted with stable machine reason codes.

Projects can be paused independently, and task admission can include an RFC 3339 deadline and repeatable adapter capability requirements:

Placeholders: none; the project, calendar, timestamp, and reason values are literal examples.

```console
garnish --data-dir .garnish-state project pause --project example --reason "project maintenance"
garnish --data-dir .garnish-state project resume --project example --reason "maintenance complete"
garnish --data-dir .garnish-state task add --project example --title "Timed work" --goal "Create result.txt" --accept "result.txt contains done" --verify-argv '["grep","-q","done","result.txt"]' --scope result.txt --non-scope "remote Git" --deadline-at 2026-07-21T17:00:00Z --requires-capability structured_output --fake-write-path result.txt --fake-write-content done
```

The continuous daemon owns and renews both the leader lease and its task claims. `TERM` or `INT` requests a bounded graceful stop that releases its claims and returns not-yet-started tasks to `ready`:

Placeholders: none.

```console
garnish --data-dir .garnish-state scheduler daemon --instance local-1 --hostname my-mac
```

By default the daemon only arbitrates and holds eligible work. `--execute-fake` additionally exercises the quota-free fake claim-to-run path: the route, claim, single-use action key, run, run lease, and project lock are bound durably before fake execution begins. Codex subscription and API execution remain separate opt-in paths. `--max-ticks` provides a bounded diagnostic run.

Placeholders: none.

```console
garnish --data-dir .garnish-state scheduler daemon --instance local-1 --hostname my-mac --execute-fake --max-ticks 1
```

Runtime supervision persists lease-fenced checkpoint decisions, cancellation intent, process termination evidence, retry budgets/backoff, and adapter circuit state. Pause/cancel decisions retain the run lease until the worker records TERM/KILL completion.

Placeholders: `RUN_ID` must be replaced with an exact `run_id` from `runtime runs`; `TASK_ID` must be replaced with an exact task `id` from `task add` or task status output.

```console
garnish --data-dir .garnish-state runtime checkpoint --run RUN_ID --provider fake --account test
garnish --data-dir .garnish-state runtime runs --task TASK_ID
garnish --data-dir .garnish-state runtime cancel --run RUN_ID --reason "user requested"
garnish --data-dir .garnish-state runtime retry-state --task TASK_ID
garnish --data-dir .garnish-state runtime retry-limit --task TASK_ID --limit 3
garnish --data-dir .garnish-state runtime circuits
```

### Phase 3 capability matrix

Schema 8 stores append-only Codex, Claude, and Antigravity capability probes. Refreshing probes only runs each CLI's version check; it does not submit an agent task or consume provider quota. Status distinguishes current, stale, and never-observed evidence.

Placeholders: none.

```console
garnish --data-dir .garnish-state agent refresh --valid-seconds 300
garnish --data-dir .garnish-state agent status
```

A missing or unsupported CLI is recorded as evidence rather than treated as healthy. The pure multi-candidate kernel hard-filters probe freshness, health, capabilities, quota headroom, and exact manual pins before applying a deterministic recorded score.

Schema 9 makes task pins durable and lets scheduler ticks/daemons consider multiple route identities. Pin changes always require a reason:

Placeholders: `TASK_ID` must be replaced with the exact task `id` returned by `task add`.

```console
garnish --data-dir .garnish-state task pin TASK_ID --adapter codex --provider codex --account default --reason "use the explicitly selected Codex subscription"
garnish --data-dir .garnish-state task unpin TASK_ID --reason "continuity no longer required"
garnish --data-dir .garnish-state scheduler tick --instance local-1 --generation 1 --candidate codex:codex:default
```

`TASK_ID` is a placeholder for the ID returned by `task add`. Candidate values are real configured identities, not fallback API permissions. A pin cannot bypass capability, health, policy, quota, or concurrency gates.

Schema 14 includes the current CodexBar usage JSON contract, append-only quota observations and collection attempts, five-minute freshness by default, atomic scheduler reservations, durable historical-usage samples, and separately attributable verifier runs. A real refresh may read provider authentication state and access the network, but it does not submit an agent task. It is always an explicit command:

Placeholders: none; `default` is the literal Garnish account label in this example.

```console
brew install --cask codexbar
codexbar --version
garnish --data-dir .garnish-state quota refresh-codexbar --provider codex --account default --source auto --valid-seconds 300
garnish --data-dir .garnish-state quota status
garnish --data-dir .garnish-state quota attempts
garnish --data-dir .garnish-state quota reservations
```

After the macOS cask install, open CodexBar and choose **Preferences → Advanced → Install CLI** before running `codexbar --version`. These commands contain no placeholders: `default` is the literal Garnish label for CodexBar's current/default account. To select a named CodexBar account, add `--collector-account ACCOUNT_LABEL`; `ACCOUNT_LABEL` is then a placeholder for the exact label in CodexBar configuration. Unknown, malformed, ambiguous, or stale provider evidence fails closed. Raw quota JSON is not retained; Garnish stores normalized surfaces and a SHA-256 payload digest. For nested provider-CLI collection, Garnish preserves a narrow non-secret runtime allowlist while excluding API keys and OAuth tokens. On Linux, the current official CLI formula is `brew install steipete/tap/codexbar`.

Historical consumption is never guessed from before/after account percentages. Trusted collectors can append deduplicated per-run evidence through `quota record-usage`; `quota samples` exposes it. Five matching evidence groups are required before an exact adapter/provider/account P90 replaces the conservative fallback. This command has no placeholders and only reads local state:

Placeholders: none.

```console
garnish --data-dir .garnish-state quota forecast --adapter codex --provider codex --account default --estimated-seconds 600 --uncertainty-percent 25
```

`quota record-usage` is currently a collector/operator ingestion contract, not an instruction to estimate percentages by hand. Its required `--evidence-id` must be a stable real run or collector identifier; `--source` must name the real evidence source. Provider-reported, collector-measured, and explicit user-reported confidence are distinct from untrusted agent output.

### CLI MVP: one Codex subscription patch

The Codex subscription lane is intentionally one task per daemon invocation. It uses saved Codex CLI authentication in the host control process, but runs `codex exec` ephemerally with JSONL output and the built-in `:read-only` permission profile. User configuration, exec-policy rule files, MCP, apps, hooks, plugins, multi-agent operation, and web search are disabled. Repository content, including `AGENTS.md`, remains untrusted model input and cannot expand authority. Codex cannot write the repository: its sole accepted result is one bounded unified Git patch, which Garnish validates against the task's exact scope and applies only to an isolated worktree. A detached verifier must pass before the task reaches `review`. These choices follow the official [Codex non-interactive mode](https://learn.chatgpt.com/docs/non-interactive-mode), [Codex permissions](https://learn.chatgpt.com/docs/permissions), and [Codex `AGENTS.md`](https://learn.chatgpt.com/docs/agent-configuration/agents-md) contracts.

#### Codex subscription setup on Ubuntu Linux and WSL2

Run the installation commands independently on each Linux or WSL2 host. The distribution `bubblewrap` package provides the preferred Codex Linux sandbox boundary. The official installer places the current Codex CLI in the user environment. These commands download software and may request `sudo`, but they do not authenticate or submit an agent task.

Placeholders: none; these commands are for Ubuntu or another Debian-family distribution using `apt`.

```console
sudo apt update
sudo apt install -y bubblewrap curl
curl -fsSL https://chatgpt.com/codex/install.sh | sh
codex --version
```

For subscription access, sign in with ChatGPT rather than an API key. Device authentication is the recommended flow for a remote Linux host or WSL2 terminal where a browser callback may not work. The command displays a URL and one-time code; follow those displayed instructions in a browser. It writes local Codex authentication state but does not submit an agent task.

Placeholders: none; do not paste the displayed one-time code into this repository or chat.

```console
codex login --device-auth
codex login status
```

On a graphical Linux session with a working browser callback, `codex login` is the official alternative to `codex login --device-auth`; use one login flow, not both. Authenticate each host independently and never copy, commit, or share `~/.codex/auth.json`. `codex login status` should report ChatGPT authentication for the subscription lane. Installation, login, version output, and login status are setup evidence only; they do not establish live Garnish execution.

First make sure `codex` is installed and already signed in with the subscription account you intend to use. These two commands do not submit a task:

Placeholders: none.

```console
codex --version
garnish --data-dir .garnish-state agent refresh --valid-seconds 300
```

Next provide quota evidence. Prefer the CodexBar refresh documented above because it expires after its configured validity window. For an operator-controlled initial test, the following manual value is explicit evidence supplied by you.

Placeholders: `90` is an example numeric value and must be replaced with the remaining percentage you actually observe immediately before the command. Do not replace it with words.

```console
garnish --data-dir .garnish-state quota set --provider codex --account default --surface five_hour --remaining-percent 90 --reserve-percent 20 --source "manual pre-test observation"
```

A manual observation is durable until you replace it and therefore does not prove freshness by itself. Re-check the real account immediately before the run, use CodexBar whenever available, and do not reuse an old manual percentage as evidence for a later task.

Create a risk-class 1 task with an exact file scope and deterministic verification command.

Placeholders: `paste-the-id-returned-by-task-add-here` must be replaced with the exact `id` printed by the preceding `task add` command. Keep the surrounding single quotes. `TASK_ID` is then a shell variable, not a placeholder in the remaining commands.

```console
garnish --data-dir .garnish-state task add --project example --title "Create result" --goal "Create result.txt containing exactly done" --accept "result.txt contains done" --verify-argv '["grep","-qx","done","result.txt"]' --scope result.txt --non-scope "all other files; Git integration" --risk-class 1
TASK_ID='paste-the-id-returned-by-task-add-here'
garnish --data-dir .garnish-state task pin "$TASK_ID" --adapter codex --provider codex --account default --reason "explicit one-task subscription test"
garnish --data-dir .garnish-state task readiness "$TASK_ID" --adapter codex --provider codex --account default
```

Read the `task readiness` JSON before continuing. This uses the scheduler's real exact-candidate, durable probe, policy, schedule, pin, and quota filters without creating a claim. `allowed` must be `true`, and the selected adapter/provider/account must be `codex`, `codex`, and `default`. The next command can consume Codex subscription quota. It accepts at most one claimed task, makes no automatic paid-API fallback, stops after that Codex task, and never retries a timeout or other uncertain process result automatically:

Placeholders: none. `I_ACCEPT_ONE_CODEX_SUBSCRIPTION_TASK` is the required literal acknowledgement.

```console
garnish --data-dir .garnish-state scheduler daemon --instance codex-local --candidate codex:codex:default --execute-codex --acknowledge-codex I_ACCEPT_ONE_CODEX_SUBSCRIPTION_TASK
```

After it returns successfully, inspect the stable review bundle. This does not commit, merge, or modify the source checkout:

Placeholders: none in this block if the `TASK_ID` shell variable was set by the earlier explicitly declared assignment.

```console
garnish --data-dir .garnish-state task review "$TASK_ID"
```

Confirm `task.status` is `review`, `verification.passed` is `true`, `handoff.changed_files` contains only the declared scope, and `integration_authorized` is `false`. Review the file named by `artifacts.patch_path`; Git integration remains your decision. If the daemon reports a timeout, cancellation, truncated output, malformed JSONL, or rejected patch, treat that attempt as uncertain or failed and do not immediately rerun it.

For the first live validation, use the dedicated smoke instead of creating Garnish state by hand. It creates a temporary Git repository and temporary database, runs exactly one subscription task against exact `result.txt` scope, verifies in a detached worktree, and proves the registered source checkout stayed unchanged. Set the first variable to the numeric percentage you actually observe immediately before the test. The example uses `90` only for the case where the observed value is 90; placeholder words such as `CURRENT_PERCENTAGE` are intentionally rejected:

Placeholders: `90` is an example numeric value and must be replaced with the percentage observed immediately before this run. `I_ACCEPT_ONE_CODEX_SUBSCRIPTION_TASK` is a required literal and must not be changed.

```console
export GARNISH_REAL_CODEX_REMAINING_PERCENT='90'
export GARNISH_ACKNOWLEDGE_CODEX_SUBSCRIPTION='I_ACCEPT_ONE_CODEX_SUBSCRIPTION_TASK'
./scripts/test-real-codex-subscription-smoke
```

Only the final script command submits the Codex task. It can consume subscription quota, has a 45-minute process ceiling, persists no raw JSONL/reasoning/stderr, has no automatic retry, and cannot fall back to a paid API. A timeout or interrupted result is uncertain: stop and inspect before considering another run. A successful run writes a private redacted receipt under `target/codex-smoke-receipts/`; it contains no authentication, prompt, model output, or quota percentage.

### Phase 4 controlled MCP registration

Schema 20 can record an MCP server trust revision but cannot launch one. Registration is default-disabled and quota-free. The command requires a project, exact server name, absolute executable path, lowercase SHA-256, source, and reason; use `--help` for the optional exact tool, host, protected secret-reference, argv, timeout, and byte-limit fields.

Placeholders: none; these are help and status commands and do not register or launch a server.

```console
cargo run --locked -- --data-dir .garnish-state mcp server-set --help
cargo run --locked -- --data-dir .garnish-state mcp server-status
```

Setting `--enabled true` records administrative eligibility and requires at least one `--tool`; it does not execute the server. There is intentionally no launch, discovery, install, or tool-call command in this slice. ADR 0013 defines the boundary.

### Phase 4 API budget control plane

Schema 20 retains the Schema 19 paid OpenAI/Anthropic API controls while adding only the non-executing MCP registration state described above. API budgets remain separate from subscription quotas, with append-only model-price evidence, per-task exact request plans, and durable bounded dispatch-attempt evidence. Configuration, fixture execution, and the read commands below cannot make a provider request or spend credit. Live scheduler execution requires a separate command-line activation described below.

Placeholders: none; the two commands ending in `--help` only print their argument contracts.

```console
cargo run --locked -- --data-dir .garnish-state api budget-status
cargo run --locked -- --data-dir .garnish-state api reservations
cargo run --locked -- --data-dir .garnish-state api attempts
cargo run --locked -- --data-dir .garnish-state api spend
cargo run --locked -- --data-dir .garnish-state api price-status
cargo run --locked -- --data-dir .garnish-state api plan-status
cargo run --locked -- --data-dir .garnish-state api price-set --help
cargo run --locked -- --data-dir .garnish-state api plan-set --help
```

These eight commands contain no placeholders; the two `--help` commands print required fields without changing state. A project budget can be configured through `api budget-set`, but configuration alone never enables spending: effective policy is independently default-deny, and no subscription-quota condition can select paid API use. Price rates are explicit integer currency micros per million tokens and are never fetched or guessed. Secret fields accept only a locator shaped as `env:NAME`, `keychain:SERVICE/ACCOUNT`, or `file:/absolute/path`; they never accept a key value. The authenticated read-only dashboard shows API budgets, exact task-plan counts, reserved and attempted requests, and settled usage under **Agents & quotas**.

Protected secret resolution is host-side and bounded. On Unix, `file:` targets must be regular files owned by the Garnish user with no group/other permissions (normally mode `0600`); symlinks are rejected. `keychain:` currently requires macOS. WSL2 and native Linux use `env:` or a protected Linux-side file. A live credential is resolved only after the complete budget/request boundary passes and either the paid scheduler acknowledgement or the separately bounded smoke-test acknowledgement is present.

API routing uses the literal adapter key `api` with provider `openai` or `anthropic`; the account is the configured Garnish account label. Paid capacity is checked against the project API budget, never subscription percentages. Every API scheduler candidate requires an exact task pin, even when it is the only candidate, so the API lane cannot act as fallback. An enabled `api plan-set` revision binds the current canonical task version, provider/account, model, implementer role, bounds, retry count, template version, and request digest without storing a duplicate prompt. The scheduler rejects missing, disabled, stale, or mismatched plans and atomically reserves currency, tokens, and request count for every allowed attempt before claiming the task. The execution boundary retries only explicitly classified rate-limit or transient provider responses within that reservation. Authentication and other terminal failures stop immediately; transport or authoritative-response uncertainty is retained and never replayed automatically.

Paid scheduler execution has a separate, conspicuous runtime gate. The following shape can make multiple chargeable requests and must not be used as a diagnostic command. `ACCOUNT` is the configured Garnish account label; every eligible task must already have the matching exact pin, active budget, effective price, and enabled exact request plan.

Placeholders: `ACCOUNT` must be replaced inside `api:openai:ACCOUNT` with the exact configured Garnish API account label. `I_ACCEPT_PAID_API_TASK_EXECUTION` is a required literal acknowledgement.

```console
garnish --data-dir .garnish-state scheduler daemon --instance paid-local --candidate api:openai:ACCOUNT --execute-api --acknowledge-paid-api I_ACCEPT_PAID_API_TASK_EXECUTION
```

The acknowledgement is session-scoped and enables only the named API provider candidates; a budget or environment variable alone remains inert. Risk-class 0 execution remains response-only: direct API responses are not persisted or applied as repository changes. The run records an honest host-direct-API attestation rather than claiming container isolation, and a completed response advances only through the task's predeclared independent verifier against the unchanged isolated worktree.

An implementation task has a separate, narrower patch boundary. It must be risk class 1, explicitly require `agent.patch_submission`, declare exact file paths in scope, use a budget that allowlists `submit_patch`, and have a current exact request plan. The daemon additionally requires both patch flags below. The provider receives only that single typed tool—never a shell or filesystem—and Garnish applies one bounded UTF-8 git diff to the isolated task worktree after structural checks. Binary patches, links, submodules, renames, copies, extra calls/arguments, and paths outside exact scope fail the task. A separate detached worktree independently verifies the resulting patch.

Placeholders: `ACCOUNT` must be replaced inside `api:openai:ACCOUNT` with the exact configured Garnish API account label. Both `I_ACCEPT_...` values are required literal acknowledgements.

```console
garnish --data-dir .garnish-state scheduler daemon --instance paid-patch-local --candidate api:openai:ACCOUNT --execute-api --acknowledge-paid-api I_ACCEPT_PAID_API_TASK_EXECUTION --execute-api-patches --acknowledge-api-patches I_ACCEPT_ISOLATED_API_PATCH_EXECUTION
```

This command can make multiple chargeable requests within the explicitly configured task and project budgets. The second acknowledgement is session-scoped and does not authorize risk classes 2 or 3, general tool execution, source-checkout writes, or automatic integration. ADR 0012 records the exact boundary.

Codex subscription and API execution are deliberately separate daemon modes; neither can authorize or fall back to the other. After a successful API implementation task, use `garnish --data-dir .garnish-state task review TASK_ID` exactly as for Codex. Confirm the verifier passed, inspect `artifacts.patch_path`, and integrate manually only after reviewing the exact changed files.

### Explicit paid API smoke test

`scripts/test-real-api-smoke` is the separately bounded one-request transport diagnostic. Do not run it as part of normal testing: it makes exactly one API request and the provider may charge it. It disables redirects, proxy inheritance, and implicit HTTP retries; reserves one request with a 32-output-token ceiling; uses a temporary database; and configures no currency price evidence. OpenAI requests set `store: false`. A timeout remains an uncertain attempted request and must not be rerun automatically.

After a successful exact test, the script writes a private redacted receipt under `target/api-smoke-receipts/`; the ignored `target/` tree is not committed. A failed or uncertain request produces no passing receipt. The receipt contains no credential, prompt, response content, raw request ID, or billing claim.

The credential stays in an environment variable and never enters a Garnish argument or repository file. These commands prepare an OpenAI smoke test but do not run it until the final script command.

Placeholders: `YOUR_REAL_OPENAI_API_KEY` must be replaced with the real OpenAI API key; `EXACT_MODEL_ID` must be replaced with the exact OpenAI model ID chosen for the test. The provider, secret reference, and `I_ACCEPT_ONE_PAID_API_REQUEST` acknowledgement are literals for this OpenAI example.

```console
export OPENAI_API_KEY='YOUR_REAL_OPENAI_API_KEY'
export GARNISH_REAL_API_PROVIDER='openai'
export GARNISH_REAL_API_MODEL='EXACT_MODEL_ID'
export GARNISH_REAL_API_SECRET_REFERENCE='env:OPENAI_API_KEY'
export GARNISH_ACKNOWLEDGE_PAID_API='I_ACCEPT_ONE_PAID_API_REQUEST'
./scripts/test-real-api-smoke
```

For Anthropic, use `ANTHROPIC_API_KEY`, provider `anthropic`, secret reference `env:ANTHROPIC_API_KEY`, and an exact Anthropic model ID. No API call is required for the current development checkpoint; run this only when explicitly choosing to spend one request.

`scripts/test-real-api-patch-smoke` is the separately ignored, one-request implementation diagnostic. It creates only a temporary repository and Garnish state, asks the selected provider for one exact `result.txt` patch, verifies that patch in a separate worktree, and proves the source checkout remains unchanged. It may incur a provider charge and is never part of normal testing. With the same provider, model, and secret-reference variables shown above, replace the response-smoke acknowledgement with:

Placeholders: none in this block; it reuses the explicitly declared provider, model, and secret-reference variables above. `I_ACCEPT_ONE_PAID_API_PATCH_REQUEST` is the required literal acknowledgement.

```console
export GARNISH_ACKNOWLEDGE_PAID_API_PATCH='I_ACCEPT_ONE_PAID_API_PATCH_REQUEST'
./scripts/test-real-api-patch-smoke
```

Successful fake execution now creates separate implementer and verifier run records. The quota-free `garnish-command-verifier:local:default` is independently selected, receives a clean detached verification worktree and its own evidence directory, and runs only the task's predeclared verification argv. It is a deterministic command verifier, not a claim of semantic agent review. Default policy requires a different verifier adapter; project policy can also require a different provider.

### Local operator interface

The initial human-facing interface is an authenticated, read-only dashboard over canonical Garnish state:

Placeholders: none.

```console
cargo run --locked -- --data-dir .garnish-state ui serve --port 7467
```

Open the exact one-time `url` printed by the command. There are no placeholders in this command. The server binds only to `127.0.0.1`, exchanges the random startup token for a strict local cookie, and displays Overview, Projects, Queue explanations, Agents & quotas, Approvals, Activity, and Settings. Stop it with `Ctrl-C`.

The URL is sensitive while that UI process is running; do not paste it into project files or shared logs. This first slice cannot modify state. Pause, resume, approval decisions, quota overrides, and emergency controls remain available through the CLI until the web mutation contract has CSRF protection, explicit confirmations, bounded typed inputs, policy checks, and durable evidence.

Operational controls are durable and emit bounded JSON. `pause` stops new claims; `emergency-stop` also releases unstarted claims and requests graceful cancellation of active runs. Neither command claims a process has stopped until its supervisor records termination evidence.

Placeholders: `NOTIFICATION_ID` must be replaced with the exact notification `id` returned by `notification list`. All reason strings are literal examples.

```console
garnish --data-dir .garnish-state ops status
garnish --data-dir .garnish-state ops pause --reason "host maintenance"
garnish --data-dir .garnish-state ops emergency-stop --reason "credential incident"
garnish --data-dir .garnish-state ops resume --reason "incident resolved"
garnish --data-dir .garnish-state ops diagnostics
garnish --data-dir .garnish-state ops backup
garnish --data-dir .garnish-state notification list
garnish --data-dir .garnish-state notification acknowledge NOTIFICATION_ID
```

Local backups use SQLite `VACUUM INTO`, pass an integrity check, receive mode `0600` on Unix, and return a SHA-256 digest. They are private state backups, not portable support-bundle exports; encrypted export remains separate work.

### Linux midpoint checkpoint

After cloning the repository on an Ubuntu or Debian host, run the quota-free Linux checkpoint as a dedicated non-root user:

Placeholders: none.

```console
./scripts/test-linux-midpoint
```

It runs formatting, lint, build and tests; exercises bounded and signal-driven daemon shutdown; verifies private state permissions; and reports rootless Podman and Docker health when installed. The 2026-07-20 VPS run passed this checkpoint and confirmed a healthy rootless Podman capability probe.

Real rootless-Podman 4.9.3 and Docker 29.6.2 conformance subsequently passed on that Ubuntu VPS. The captured runtime and attestation evidence is in [`docs/phase-2-linux-container-conformance.md`](docs/phase-2-linux-container-conformance.md).

The hardened Podman sandbox lifecycle is a separate, explicit opt-in because it needs a real local image. These commands pull Alpine once, derive its real digest automatically, and then run with further pulls and container networking disabled:

Placeholders: none; `PODMAN_IMAGE` is populated automatically from the pulled image digest.

```console
podman pull docker.io/library/alpine:latest
PODMAN_IMAGE="$(podman image inspect docker.io/library/alpine:latest --format '{{index .RepoDigests 0}}')"
GARNISH_REAL_PODMAN_IMAGE="$PODMAN_IMAGE" ./scripts/test-podman-conformance
```

The same environment variable adds this conformance test to `test-linux-midpoint` and therefore to the WSL2 bundle. Without it, those bundles report the real-container test as skipped. No agent subscription or API is used.

Docker has an equivalent opt-in test. These commands pull Alpine into Docker's separate local image store, derive the real digest, and run it:

Placeholders: none; `DOCKER_IMAGE` is populated automatically from the pulled image digest.

```console
docker pull docker.io/library/alpine:latest
DOCKER_IMAGE="$(docker image inspect docker.io/library/alpine:latest --format '{{index .RepoDigests 0}}')"
GARNISH_REAL_DOCKER_IMAGE="$DOCKER_IMAGE" ./scripts/test-docker-conformance
```

Setting both environment variables when running `scripts/test-linux-midpoint` runs both real backends. Docker and rootless Podman retain separate local image stores, container namespaces, and lifecycle tests.

### WSL2 exit bundle

Use a current rustup-managed Rust toolchain and keep the checkout in the WSL2 Linux filesystem, not under `/mnt/c` or another Windows drive mount. The project requires Rust 1.97 or newer; Rust edition 2024 is supported normally on WSL2 when the compiler is current.

Placeholders: none.

```console
./scripts/test-wsl2-exit
```

The bundle consumes no provider quota. It runs the Linux midpoint, verifies default denial of Windows-mounted project roots, requires a healthy rootless Podman or Docker runtime, exercises operational backup/status, and checks private Linux permissions. It passed on Ubuntu 24.04 under WSL2 with rootless Podman on 2026-07-20; the captured evidence and cgroup fallback caveat are in [`docs/phase-2-wsl2-exit.md`](docs/phase-2-wsl2-exit.md).

### Phase 4 portability checkpoint

After updating the checkout on either the Linux VPS or WSL2, run this exact command as the existing non-root development user:

Placeholders: none.

```console
./scripts/test-phase4-portability
```

There are no placeholders and no additional packages or container images are required. The script removes OpenAI, Anthropic, and Codex live-test credentials, selectors, acknowledgements, receipt overrides, and real-container selectors from the test environment. It checks command-placeholder declarations, formatting, strict lint, and the complete fixture-only suite. It automatically distinguishes native Linux from WSL2 and rejects a WSL checkout under `/mnt/<drive>`. The final Codex/CLI slice passed on native Linux and WSL2 on 2026-07-21; combined macOS, Linux, and WSL2 evidence is recorded in [`docs/phase-4-portability-checkpoint.md`](docs/phase-4-portability-checkpoint.md).

## Repository authority

The user manages branches and commits for this repository. Agents may edit the current checkout when explicitly asked, but must not create or switch branches, commit, push, open pull requests, merge, or alter remotes.
