# TB-0 as-is/to-be gap matrix

Status: complete source-level inventory at the TB-0 pivot. “Reuse” means preserve the tested control-plane capability behind the new interface; it does not mean the current operator workflow is retained.

## 1. Current command disposition

| Current command | Disposition | To-be owner |
| --- | --- | --- |
| `init` | Replace output/initialisation semantics; retain native path and database setup | `init` |
| `doctor` | Extend and stabilize report; keep all default checks quota-free | `doctor` |
| `project add`, `project list` | Reuse repository validation/storage; add lifecycle, affinity, calendar, backlog, and stable envelope | `project add|list|show` |
| `project pause`, `project resume` | Reuse pause primitives; replace boolean-only state with project lifecycle/checkpoint semantics | `project pause|resume` |
| `project link`, `project links` | Retain as advanced until project dependency UX is designed | `advanced project ...` |
| `task add`, `list`, `show`, `dependency` | Retain storage/dependency engine; hide routine manual task construction | `objective ...`; `advanced task ...` |
| `task review` | Reuse evidence assembly; replace task-ID integration UX | `project review` |
| `task readiness`, `route` | Reuse hard filters and rationale | `route explain`; internal scheduler |
| `task pin`, `unpin` | Retain only as diagnostic recovery override | `advanced task pin|unpin` |
| `task run` | Replace as routine entry point; keep fake/compatibility diagnostics | internal service; `advanced task run` |
| `task complete` | Replace manual internal completion | objective/review result lifecycle; `advanced task complete` |
| `quota set` | Retain as fake/development and low-level collector input | `advanced quota set` |
| `quota override` | Reuse append-only operator assertion with agent-profile identity | `quota override|clear-override` |
| `quota refresh-codexbar` | Reuse collector boundary behind configured agent profile | `quota refresh` |
| `quota record-usage`, `forecast`, `samples`, `attempts`, `reservations` | Reuse accounting/evidence; move detailed surfaces to diagnostics | `quota status|explain`; `advanced quota ...` |
| `quota status` | Replace bare rows with project/account summary and stable envelope | `quota status` |
| `api budget-set`, `budget-status` | Reuse project budget revisions; expose as project policy/settings rather than routine family | `project configure`, `policy`, `quota`; advanced detail |
| `api reservations`, `attempts`, `spend` | Reuse accounting and uncertainty evidence | `quota status`, `events`; advanced detail |
| `api price-set`, `price-status` | Retain as managed/admin compatibility until settings UX exists | `advanced api ...` |
| `api plan-set`, `plan-status` | Remove from normal workflow; generate exact plans internally from objective/project policy | internal route/run; `advanced api ...` |
| `mcp server-set`, `server-status` | Preserve default-deny registration; no normal executable MCP until later acceptance | project policy; `advanced mcp ...` |
| `schedule configure` | Replace mutable W/O profile with append-only W/O/B calendar revisions | `calendar set` |
| `schedule assign` | Reuse assignment with project versioning | `calendar assign` |
| `schedule exception` | Replace W/O-only row with revisioned W/O/B exception | `calendar exception set|remove` |
| `schedule evaluate` | Replace task operand with project/objective explanation | `calendar preview`, `project status` |
| `schedule preview` | Reuse evaluation but remove manually selected route target | `calendar preview`, `route explain` |
| `scheduler daemon` | Reuse leader/tick foundations; remove user route/acknowledgement construction | `service run`; internal service |
| `scheduler register`, `acquire-leader`, `heartbeat`, `tick`, `recover`, `stop`, `wakes` | Retain as internal diagnostics/recovery only | `service`, `status`, `maintenance`; `advanced scheduler ...` |
| `runtime runs`, `checkpoint`, `cancel`, `retry-state`, `retry-limit`, `circuits` | Reuse supervision/retry/circuit data; hide run IDs in normal operation | project/service lifecycle; `advanced runtime ...` |
| `ops status`, `pause`, `resume`, `emergency-stop`, `diagnostics`, `backup` | Reuse and stabilize; add dry-run and restore check | `ops ...` |
| `notification list`, `acknowledge` | Reuse local inbox; add event coverage, outbox, delivery state, filters/configuration | `notification ...` |
| `agent probe`, `refresh`, `status` | Reuse probe logic; bind results to named profiles/accounts | `agent probe|show|list`, `quota refresh` |
| `agent invocation` | Retain as safe argv diagnostic only | `advanced agent invocation` |
| `approval request`, `consume` | Internal broker operations; never normal human commands | approval service; `advanced approval ...` |
| `approval approve`, `deny` | Replace names and bind displayed digest/version | `approval allow|deny` |
| `ui serve` | Preserve read-only diagnostic until mutation parity is accepted | `advanced ui serve`; later service/UI command |
| standalone `recover` | Fold into automatic service recovery and maintenance explanation | `service`, `maintenance reconcile`; advanced recover |

## 2. Current service/module disposition

| Source area | Reusable foundation | Missing to-be capability |
| --- | --- | --- |
| `db.rs` | Transactional SQLite migrations, backups, events, tasks, scheduler, quota, approvals, API/MCP state | Schema 21–25, project/objective lifecycle, settings revisions, outbox, sandbox/cleanup/review ownership |
| `domain.rs` | Typed task/calendar/quota/route/run records | W/O/B day kind, project affinity/status, objectives, stable public DTOs, normalized new lifecycles |
| `service.rs` | Project/task/quota/scheduler/run/verifier/API orchestration | Project-first supervisor, automatic objective generation, cross-agent handoff, result apply/discard, cleanup orchestration |
| `schedule.rs` | Timezone-safe evaluation and next-wake logic | System `B` days, project inheritance, active-boundary checkpoint behaviour |
| `routing.rs` | Hard gates, scoring components, candidate evidence | Named profiles, subscription-first policy, project-wide continuous routing/handoff |
| `quota.rs` | Multi-surface evidence, reserve/forecast, CodexBar parsing | Profile binding, continuous refresh, UI summary, Claude sources and reset-aware handoff |
| `policy.rs` | Effect classes and allow/approval/deny foundation | Layered versioned resolver, field provenance, real structured agent-action broker |
| `adapters.rs` | Version probes, invocation contracts, fake adapter, narrow Codex path | Real attested-container Codex/Claude sessions, structured approvals, checkpoints and handoff |
| `process.rs` | Bounded process execution/cancellation primitives | Container/session integration and durable descendant termination evidence |
| `git.rs` | Isolated worktrees and source invariants | Review-result apply/discard and automatic owned cleanup/quarantine |
| `evidence.rs` | Manifests, handoffs, patches, verification artifacts | Canonical handoff/review lifecycle and retention ownership |
| `projections.rs` | Generated project evidence and conflict awareness | Project-first projections and no routine TASKS/HANDOFF operator dependency |
| `notifications.rs` | Fake notifier interface | Transactional outbox, native desktop adapters, retries/deduplication/quiet hours |
| `secrets.rs` | Env/file/Keychain references and secret wrapper | Secret CLI, metadata lifecycle, Linux Secret Service, rotation and isolated subscription auth |
| `api_providers.rs`, `api_pricing.rs` | Bounded direct transports, exact accounting and price evidence | Internal project route integration without per-task operator plans |
| `web_ui.rs` | Authenticated read-only loopback presentation | Shared project/status/approval/settings mutations with CSRF and CLI parity |
| `main.rs` | Clap parsing and JSON serialization | Contracted normal/advanced grammar, human mode, stable envelope and exit taxonomy |

## 3. Product gaps ordered by dependency

1. Stable CLI/JSON contract and schema migration fixtures.
2. Project/objective/calendar/settings data model.
3. Project-first service loop and automatic cleanup with fake agents.
4. Structured approval, transactional notifications, and protected secret metadata/projection.
5. Attested-container Codex execution and provider-specific auth isolation.
6. Claude execution and subscription handoff.
7. Optional paid API and additional-agent routing behind project policy.
8. Mutable web parity, service installation, and release acceptance.

No lower item may be called complete by exercising only an existing task-oriented compatibility command.

