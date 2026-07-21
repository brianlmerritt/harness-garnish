# TB-0 schema and data migration plan

Status: design plan only. Schema 20 remains current. TB-1 must implement each migration with pre-migration backup, fixture upgrade tests, integrity checks, and a clean-install equivalence test before increasing `SCHEMA_VERSION`.

## 1. Migration principles

- Never reinterpret historical rows in place when their old semantics remain useful evidence.
- Append new revision/state tables and preserve stable existing IDs wherever the meaning is unchanged.
- Migrate under one immediate SQLite transaction after creating and verifying the existing automatic backup.
- Fail before changing `user_version` on an unknown value, invalid legacy data, path ownership conflict, or unrepresentable calendar/project state.
- A new binary must produce the same schema from clean installation and sequential upgrade.
- Normal/portability migration tests use temporary state and no provider, container, notification, or keyring effect.
- The first project-supervisor service start after upgrade is inert: legacy projects are not silently made eligible for new autonomous work.
- An older binary cannot open the newer database. Rollback means stopping Garnish and restoring the verified pre-migration backup; it is not a lossy down-migration.

## 2. Proposed schema sequence

The sequence is deliberately split so each acceptance boundary can be exercised independently. Exact version numbers are reserved by this plan but become current only when the associated code lands.

### Schema 21 — projects, objectives, and `W`/`O`/`B` calendars

Add:

- `calendar_profile_revisions`: immutable name, timezone, seven-character pattern, version, source, reason, supersedes ID, timestamps;
- `calendar_exception_revisions`: immutable profile revision/date/`W|O|B` kind/reason/supersession rows;
- project lifecycle status, project affinity `work|non_work|both`, active calendar revision, backlog source, integration policy, and version;
- `objectives`, `objective_dependencies`, and an optional `tasks.objective_id` link;
- project-status transition events and indexes for active/scheduled projects.

Migration rules:

- Copy every existing calendar profile as revision 1. Existing `W`/`O` patterns are a valid subset of `W`/`O`/`B`.
- Copy existing `W`/`O` exceptions unchanged; future rows permit `B`.
- Do not silently replace an existing weekly pattern with `WWWOOBB`. New installations default to the explicitly configured setup value; `init` proposes `WWWOOBB` only as the current product default and shows it before persistence.
- A scheduler-paused legacy project becomes `paused`; every other legacy project becomes `stopped`, preventing new automatic work merely because the binary was upgraded.
- Infer a provisional project affinity only when all legacy non-`B` tasks agree. Mixed work/off legacy tasks yield project affinity `both`. Preserve each old task affinity as a compatibility-only narrowing override until that task is terminal.
- Create one `legacy_task` objective for each non-superseded task, preserving title, goal, acceptance, priority, status mapping, and task ID linkage. These objectives are visible but clearly sourced from the compatibility migration.
- No legacy review task is marked integrated. A review result must be reconstructed only when existing manifest, patch, base, verification, and source-immutability evidence all validate; otherwise status is blocked with an explicit recovery action.

### Schema 22 — layered settings, policy, agents, and secret metadata

Add:

- `setting_revisions` with typed canonical JSON, scope kind/ID, key path, managed/delegable flags, source, reason, actor, supersedes ID, and timestamps;
- `policy_revisions` with scope, document/hash, source, reason, actor, supersedes ID, and timestamps;
- `agent_profiles` and append-only `agent_profile_revisions` for kind, account label, executable, version range, model policy, quota source, sandbox/auth requirements, and enabled state;
- `secret_metadata` and `secret_versions` containing only protected backend locators and lifecycle metadata;
- effective-setting cache/projection tables only if measurements show resolution requires them; canonical revisions remain authoritative.

Migration rules:

- Translate current hard-coded defaults into explicit built-in source revisions, not user-authored values.
- Convert capability probe adapter names into disabled/unconfirmed agent-profile suggestions. A probe never proves account identity or authorises execution.
- Convert API budget secret references into secret metadata references without resolving or copying the value. Invalid/unsupported locators disable the affected API budget and create a notification.
- Existing project policy remains the baseline managed-safe default; migration cannot broaden network, Git, MCP, secret, paid API, or integration authority.

### Schema 23 — approvals and notification outbox

Add/replace:

- approval project/run identity, canonical action version, bounded summary, policy/sandbox revision, status enum, invalidation/cancellation reason, and optimistic version;
- `notification_intents`, `notification_deliveries`, and per-user inbox state;
- event-to-notification deduplication/grouping keys;
- delivery-channel configuration revisions.

Migration rules:

- Historical approvals remain audit rows. Pending legacy approvals are invalidated because they lack the complete new canonical action/sandbox binding.
- Historical local notifications become acknowledged or unread inbox entries according to `acknowledged_at`; no delivery attempts are synthesized.
- Existing notification text is treated as bounded historical content and never expanded with task logs or secret references.

### Schema 24 — sandbox, handoff, review result, and cleanup ownership

Add:

- `sandbox_specs`, `sandbox_instances`, and immutable `sandbox_attestations`;
- canonical `handoffs` with content hash and lifecycle status;
- `review_results` with patch/base/scope/verification/source-invariant digests and disposition;
- `cleanup_plans`, exact cleanup targets, attempts, ownership proof, plan digest, retention eligibility, and outcome;
- run lifecycle normalization and uncertain-action identity;
- explicit source-checkout identity and observed Git invariants.

Migration rules:

- Existing run manifests remain evidence and are linked when hashes validate; they do not become secure-container attestations retroactively.
- Existing worktree paths and branches are inventory candidates only. Migration performs no deletion. The first maintenance preview classifies owned, unowned, missing, dirty, and ambiguous resources.
- Existing handoff artifacts are imported only after schema/hash/bounds validation; otherwise they remain external historical artifact references.
- ADR 0014 runs remain labelled host-direct/read-only-patch and never migrate to secure-container status.

### Schema 25 — service and contract activation

Add:

- service instance/health and command-contract version records;
- project scheduler cursor and objective-generation idempotency keys;
- stable result/event cursors required by `status`, `events`, and UI parity;
- activation marker proving schema-21–24 validation and operator acknowledgement of project settings.

Activation rules:

- Upgrade completes with the service paused.
- `doctor` reports each project requiring affinity/calendar/backlog/policy review.
- A project can enter `active` only after its required effective settings validate.
- Compatibility commands remain available beneath `advanced` throughout TB-1/TB-2.

## 3. Data checks before upgrade

The migration preflight must report and fail closed on:

- duplicate or non-canonical project roots, missing repositories, or WSL2 Windows-mounted roots denied by policy;
- invalid calendar timezone/pattern/date or inconsistent profile assignment;
- tasks referencing missing projects/dependencies, dependency cycles, or unknown statuses;
- active claims/leases/runs that cannot be safely stopped or recovered;
- worktrees/branches whose ownership cannot be proven;
- pending API dispatch attempts or `uncertain` actions;
- invalid secret references, malformed policy/config JSON, or secret canaries in durable fields;
- corrupt SQLite integrity, foreign keys, event digest chain, artifact hashes, or pre-migration backup.

The migration command must support a read-only plan that lists exact transformations, blockers, backup path proposal, and expected new schema without creating the backup or modifying state.

## 4. Acceptance fixtures

Required quota-free fixtures include:

1. clean schema-20 database with no projects;
2. `WWWWWOO` calendar and one unpaused legacy project;
3. paused project with work-only tasks;
4. project with mixed work/off/both legacy task affinities;
5. review task with complete valid artifact evidence;
6. review task with missing or mismatched evidence;
7. pending legacy approval and unread notification;
8. API budget with environment, Keychain, and file secret references using fake names only;
9. dirty/missing/unowned worktree inventory;
10. interrupted upgrade proving transaction rollback and intact backup;
11. sequential upgrade and clean-install schemas compared through normalized `sqlite_schema` and required seed data;
12. restored schema-20 backup successfully reopened by the old compatibility binary fixture.

Every fixture asserts that project source checkouts, Git refs, provider quota, API budgets, keyrings, notification systems, and container runtimes are untouched.

