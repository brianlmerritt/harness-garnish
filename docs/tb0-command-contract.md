# TB-0 CLI and JSON contract

Status: frozen design contract for TB-0. It is intentionally ahead of the current executable. The machine-readable source is [`contracts/tb0-cli-v1.json`](contracts/tb0-cli-v1.json), and quota-free tests validate it before TB-1 changes runtime behaviour.

## 1. Interface boundary

The normal CLI is project-first. Internal tasks, scheduler instances, claims, leases, direct runtime controls, API request plans, and MCP registration remain available only through `garnish advanced` during the compatibility period.

The normal top-level families are exactly:

`init`, `doctor`, `status`, `service`, `config`, `calendar`, `project`, `objective`, `agent`, `quota`, `route`, `approval`, `notification`, `secret`, `policy`, `events`, `ops`, and `maintenance`.

`advanced` is a compatibility gateway and is not a normal family. The existing `task`, `api`, `mcp`, `schedule`, `scheduler`, `runtime`, `ui`, and standalone `recover` surfaces move beneath it without initially changing their legacy arguments or response bodies. Existing `project`, `quota`, `agent`, `approval`, `notification`, and `ops` compatibility operations that do not match this contract also remain reachable beneath `advanced`.

No compatibility command is removed until its replacement has machine evidence and an announced deprecation window. Compatibility does not make a command part of the intended user workflow.

## 2. Global grammar

```text
garnish [--data-dir PATH] [--output human|json] [--no-color] [--quiet] COMMAND
```

- Interactive terminals default to `human`; non-interactive consumers must request `--output json` rather than infer formatting.
- `--data-dir` retains `GARNISH_DATA_DIR` compatibility. No other security-relevant setting is silently sourced from an environment variable unless its configuration contract names that source.
- `--quiet` suppresses progress, not the final result or errors.
- `--no-color` affects human output only.
- Material mutations accept `--dry-run`. Dry-run performs validation and policy evaluation but makes no durable, filesystem, process, provider, notification, or secret-store change.
- IDs and slugs are accepted wherever the operand is named `project`, `objective`, `agent`, `approval`, `notification`, `secret`, `event`, or `result`. Ambiguous names fail with exit code 5 and list non-secret candidates.
- Times use RFC 3339. Local dates use `YYYY-MM-DD`. Calendar timezones use IANA identifiers.
- Repeatable options such as `--accept` preserve input order.
- Secret values are never positional operands or ordinary option values.

## 3. Normal command grammar

The notation below is descriptive, not a copyable shell recipe: uppercase words are operands, brackets are optional, and `...` means repeatable.

### Bootstrap and system

| Command | Contract |
| --- | --- |
| `init [--calendar-pattern PATTERN] [--timezone ZONE] [--execution-backend BACKEND] [--dry-run]` | Initialise private state, config, default calendar, and safe execution preference. It never logs into an agent or sends a provider request. |
| `doctor [--check CHECK]...` | Run bounded, quota-free capability and permission checks. Network/provider tasks require a separate future live diagnostic boundary. |
| `status [--project PROJECT] [--watch-seconds N]` | Return the complete system or project operational summary. |
| `service install|uninstall|start|stop|restart|status|run` | Manage the local scheduler service. Material actions support dry-run; `run` is the foreground form and may take `--max-cycles`. |

### Settings and calendars

| Command | Contract |
| --- | --- |
| `config show [PATH]` | Show non-secret effective configuration. |
| `config set PATH VALUE [--reason TEXT] [--dry-run]` | Set a global configuration value. Secret-bearing paths are rejected. |
| `config explain PATH` | Show value, source revisions, overrides, ceilings, delegability, and restart/reschedule effect. |
| `config validate [--file FILE]` | Validate effective or proposed configuration without applying it. |
| `config edit [--editor PROGRAM] [--dry-run]` | Edit a temporary non-secret document, validate it, show a semantic diff, then apply only after confirmation. |
| `config export --file FILE [--dry-run]` | Export non-secret configuration only. |
| `calendar list|show CALENDAR` | Inspect calendars. |
| `calendar set CALENDAR --timezone ZONE --pattern PATTERN [--reason TEXT] [--dry-run]` | Append a calendar revision. `PATTERN` is exactly seven `W`, `O`, or `B` characters. |
| `calendar exception set CALENDAR DATE KIND --reason TEXT [--dry-run]` | Add or replace a dated `W`/`O`/`B` exception. |
| `calendar exception remove CALENDAR DATE --reason TEXT [--dry-run]` | Remove a dated exception through a new revision. |
| `calendar preview CALENDAR [--from DATE] [--days N] [--project PROJECT]` | Explain day classes and optional project eligibility. |
| `calendar assign PROJECT CALENDAR [--reason TEXT] [--dry-run]` | Assign a calendar to a project. |

### Projects, objectives, and review

| Command | Contract |
| --- | --- |
| `project add PATH [--slug SLUG] [--title TEXT] [--affinity work|non-work|both] [--calendar CALENDAR] [--dry-run]` | Register one repository. Defaults are derived and shown before persistence. |
| `project list [--status STATUS]` | List projects without exposing internal tasks. |
| `project show PROJECT` | Show identity and effective project configuration. |
| `project configure PROJECT SETTING VALUE [--reason TEXT] [--dry-run]` | Append one project setting revision. |
| `project start PROJECT [--reason TEXT] [--dry-run]` | Make the project eligible for supervision; it does not promise immediate execution. |
| `project pause PROJECT --reason TEXT [--dry-run]` | Stop new work and checkpoint active work at the next safe boundary. |
| `project resume PROJECT [--reason TEXT] [--dry-run]` | Restore eligibility subject to calendar, policy, quota, and health. |
| `project stop PROJECT --reason TEXT [--dry-run]` | Cancel/checkpoint active work according to policy and enter stopped state. |
| `project status PROJECT` | Explain objectives, current run, route, quota, approval, cleanup, next action, and next wake. |
| `project review PROJECT [--result RESULT]` | Show verified pending results and bounded evidence. |
| `project apply RESULT [--reason TEXT] [--dry-run]` | Apply the verified patch to a clean registered checkout only when base and tracked-state checks pass. It never commits, pushes, merges, or deploys. |
| `project discard RESULT --reason TEXT [--dry-run]` | Reject a result, preserve the audit record, and schedule owned execution cleanup. |
| `project archive PROJECT --reason TEXT [--dry-run]` | Stop supervision and retain project history. |
| `project remove PROJECT --reason TEXT [--dry-run]` | Remove registration only after a preview proves no active work or unretained evidence. It never deletes the source repository. |
| `objective add PROJECT --title TEXT --goal TEXT --accept CRITERION... [--priority N] [--depends-on OBJECTIVE]... [--dry-run]` | Add user-visible work to the first supported backlog source. |
| `objective list [--project PROJECT] [--status STATUS]` | List objectives. |
| `objective show OBJECTIVE` | Show the objective plus summarized internal progress. |
| `objective complete OBJECTIVE [--reason TEXT] [--dry-run]` | Record explicit completion where no pending result remains. |
| `objective cancel OBJECTIVE --reason TEXT [--dry-run]` | Cancel future work and safely stop active work. |

The first backlog source is Garnish-local objectives. In TB-1 each objective deterministically creates one internal implementation task plus verifier/cleanup work. Planning-agent decomposition and issue-provider import are deferred until their own trust and conflict contracts exist.

The default integration policy is `review`. Verified changes become a result artifact; the execution worktree can be removed after the patch, base, manifest, and verification evidence are durable. `project apply` is the normal explicit integration action. This prevents failed or completed tasks from littering the repository with permanently occupied worktrees and branches.

### Agents, quota, and routing

| Command | Contract |
| --- | --- |
| `agent add NAME --kind codex|claude --account ACCOUNT [--executable FILE] [--model POLICY] [--quota-source SOURCE] [--dry-run]` | Register non-secret profile metadata. It never accepts an auth token. |
| `agent list [--kind KIND]`, `agent show AGENT` | Inspect profiles and health. |
| `agent probe [AGENT]` | Perform quota-free executable/version/capability/auth-readiness probes. It does not submit agent work. |
| `agent configure AGENT SETTING VALUE [--reason TEXT] [--dry-run]` | Append a profile setting revision. |
| `agent remove AGENT --reason TEXT [--dry-run]` | Disable/remove a profile after dependency and active-run checks. |
| `quota refresh [--agent AGENT]` | Invoke configured quota collectors only; it never submits coding work. |
| `quota status [--agent AGENT] [--project PROJECT]` | Show all surfaces, evidence freshness, reserve, forecast, reset, and uncertainty. |
| `quota explain AGENT [--project PROJECT]` | Explain headroom and project routing impact. |
| `quota override AGENT SURFACE --remaining-percent N --reason TEXT [--expires-at TIME] [--dry-run]` | Append an explicit operator assertion. |
| `quota clear-override AGENT SURFACE --reason TEXT [--dry-run]` | End an override without rewriting observations. |
| `route explain PROJECT [--objective OBJECTIVE] [--at TIME]` | Explain every hard filter and score without claiming work. |

Subscription routes are considered before paid APIs by default. An API can become a candidate only through project policy with an enabled budget, allowed model/role/tools, protected secret reference, current pricing evidence where money is limited, and any required exact approval. Low subscription quota alone is not API consent.

### Approvals, notifications, secrets, and policy

| Command | Contract |
| --- | --- |
| `approval list [--project PROJECT] [--status STATUS]`, `approval show APPROVAL` | Inspect bounded pending or historical approval records. |
| `approval allow|deny APPROVAL [--reason TEXT] [--dry-run]` | Decide only the displayed canonical action digest. Allow is single-use and expiring. |
| `notification list [filters]`, `notification show NOTIFICATION`, `notification acknowledge NOTIFICATION [--dry-run]` | Operate the durable local inbox. Acknowledgement is not approval. |
| `notification configure SETTING VALUE`, `notification mute --until TIME --reason TEXT`, `notification unmute`, `notification test --channel CHANNEL` | Configure or test delivery. Material forms support dry-run. Critical security and emergency-stop events cannot be discarded by quiet hours. |
| `secret add NAME --backend BACKEND --provider PROVIDER --account ACCOUNT --purpose PURPOSE [--file FILE] [--dry-run]` | Read a secret from a private TTY or protected file descriptor/file. Ordinary argv input is forbidden. |
| `secret list`, `secret show SECRET`, `secret test SECRET` | Return metadata and health only. `test` may perform a bounded protected-store read but never prints the value. |
| `secret rotate SECRET [--file FILE] [--reason TEXT] [--dry-run]`, `secret remove SECRET --reason TEXT [--dry-run]` | Rotate/revoke through the selected backend and invalidate dependent readiness. |
| `policy show`, `policy validate`, `policy explain PATH`, `policy set PATH VALUE` | Inspect and revise effective policy. Project/agent scope is explicit; denies and managed ceilings win. |

The initial notification delivery boundary is local inbox plus best-effort native desktop notification. Quiet hours delay non-critical desktop delivery but never delay canonical event/approval creation. Email, chat, and remote approval remain later opt-in adapters.

The initial secret backend order is macOS Keychain, Linux Secret Service when a usable session exists, and an explicit user-owned mode-`0600` protected file for headless Linux/WSL2. Subscription CLI authentication is not copied from the host profile. TB-3 must use a Garnish-owned, provider-specific authentication broker/profile that is isolated from coding shells; until that boundary passes, ADR 0014 remains the honest Codex compatibility fallback and Claude live execution remains unavailable.

### Evidence, operations, and maintenance

| Command | Contract |
| --- | --- |
| `events list [filters]`, `events show EVENT` | Inspect bounded canonical summaries and cursors; no raw reasoning or secrets. |
| `ops status|diagnostics` | Inspect control-plane health. |
| `ops pause --reason TEXT`, `ops resume`, `ops emergency-stop --reason TEXT` | Control all new/active work with durable events and dry-run support. |
| `ops backup --file FILE`, `ops restore-check --file FILE` | Create a private verified backup or inspect one without restoring it. |
| `maintenance preview [--project PROJECT] [--older-than TIME]` | Produce an immutable cleanup plan and digest. |
| `maintenance cleanup --plan PLAN [--reason TEXT] [--dry-run]` | Consume exactly one unchanged cleanup plan. |
| `maintenance reconcile [--project PROJECT] [--dry-run]` | Detect and safely reconcile Garnish-owned containers, worktrees, branches, and artifacts. |
| `maintenance integrity` | Check SQLite, artifact hashes, source-checkout invariants, and ownership records. |

Default retention decisions for the project-supervisor MVP are:

- containers and detached verifier worktrees: remove immediately after durable terminal evidence;
- clean implementation worktrees: remove immediately after patch/evidence capture;
- dirty failed worktrees: capture a bounded patch and quarantine manifest, then remove the worktree; retain the quarantine artifact for seven days;
- review-result patches/manifests/verifications: retain until applied/discarded, then 30 days;
- bounded event, approval, route, quota, and summary evidence: retain 180 days;
- verbose redacted process output: retain seven days unless a security event requires a separately approved hold;
- Garnish-owned branches: remove after captured commit/patch/base verification proves no unexpected commits; otherwise quarantine and notify rather than delete.

## 4. JSON contract

Every normal `--output json` success emits exactly one UTF-8 JSON object on stdout:

```json
{
  "schema": "garnish.cli/v1alpha1/project_status",
  "contract": "garnish.cli/v1alpha1",
  "generated_at": "2030-01-02T03:04:05Z",
  "data": {},
  "warnings": []
}
```

The `schema`, `contract`, `generated_at`, `data`, and `warnings` keys are mandatory. Resource-specific required `data` fields are frozen in the machine-readable contract. Fields may be added only within the same alpha contract when consumers are required to ignore unknown fields; removals, type changes, semantic changes, and enum narrowing require a new contract version.

List results use named arrays rather than a bare top-level array. Times are RFC 3339 strings, percentages are JSON numbers in `0..=100`, money uses integer minor/micro units plus an explicit currency, digests are lowercase algorithm-labelled strings, and absent values are `null` rather than magic strings.

JSON errors emit no stdout and exactly one object on stderr:

```json
{
  "schema": "garnish.cli/v1alpha1/error",
  "contract": "garnish.cli/v1alpha1",
  "code": "conflict",
  "message": "project name is ambiguous",
  "details": {},
  "retryable": false
}
```

Human errors remain concise stderr text. Neither mode exposes secret values, private chain-of-thought, raw provider bodies, or unbounded process output.

## 5. Exit codes

| Code | Stable meaning |
| --- | --- |
| 0 | Success, including a successful dry-run. |
| 2 | CLI usage or syntax error. |
| 3 | Input/configuration validation failed. |
| 4 | Requested resource not found. |
| 5 | Version, ownership, base, plan-digest, or ambiguity conflict. |
| 6 | Policy, approval, budget, calendar, sandbox, or authority denied the operation. |
| 7 | Required local capability, agent, backend, credential, or collector is unavailable. |
| 8 | An external/provider action may have happened; automatic retry is unsafe. |
| 9 | Internal invariant or storage failure. |

Status-like commands return code 0 when they successfully report an unhealthy or blocked state. Scripts inspect the documented status field. A failed probe returns 0 when the report itself is complete, unless the command specifically requested one required check and could not execute it.

## 6. Agent execution decisions closed by TB-0

- Codex's first structured target is a version-pinned app-server/structured-event adapter behind Garnish's adapter contract. If a supported version cannot provide the required structured approval and lifecycle semantics, it is ineligible for autonomous container execution.
- Claude Code's first structured target is its version-pinned stream JSON and permission-prompt integration behind the same canonical action broker.
- Terminal prompt scraping, generic keystroke injection, and automatically answering an agent's interactive prompt are forbidden.
- Garnish owns container construction and attestation. Agent of Empires may supervise sessions and lifecycle only through its versioned adapter; AoE defaults never establish sandbox authority.
- Optional working-hour windows are not part of the first project-supervisor MVP. Calendar day class changes cause checkpoint/pause at the next safe boundary.
- The default Git integration policy is review plus explicit `project apply`; commit, push, PR, merge, and deployment remain separately authorised future effects.

