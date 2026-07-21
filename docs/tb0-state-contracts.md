# TB-0 state-machine contracts

Status: to-be state contract for the project-supervisor boundary. Existing schema-20 task/run transitions remain authoritative for the compatibility build until the migrations and services described here are implemented and accepted.

## 1. Common transition rules

Every material transition:

1. reads a named resource and expected version;
2. validates actor authority, effective settings/policy, calendar, quota/budget, dependencies, sandbox requirements, and current lifecycle state as applicable;
3. appends the new revision/state, canonical event, notification-outbox row, and required ownership/cleanup work in one SQLite transaction;
4. returns the new version plus the event sequence;
5. fails with a conflict rather than partially applying when the expected version or action digest changed.

Commands never mutate status through an unrestricted setter. Recovery uses named recovery transitions. Human-readable labels are not machine state.

## 2. Project lifecycle

States: `registered`, `active`, `pausing`, `paused`, `stopping`, `stopped`, `blocked`, `completed`, `archived`.

| From | To | Trigger and invariant |
| --- | --- | --- |
| none | registered | `project add`; repository identity and ownership checks pass. |
| registered, paused, stopped, blocked | active | `project start`/`resume` or resolved blocker; calendar does not need to be currently eligible, because active means supervised rather than executing now. |
| active | pausing | Operator pause or eligibility boundary; no new internal work may be claimed. |
| pausing | paused | Active work reached a safe checkpoint or bounded termination completed. |
| active, pausing, paused, blocked | stopping | Operator stop; cancellation/checkpoint and cleanup become required. |
| stopping | stopped | No owned execution remains active and required evidence is durable. |
| active | blocked | No safe route/action exists and an operator/external change is required. |
| blocked | active | The exact blocker is resolved and policy still allows supervision. |
| active, paused, stopped | completed | All non-cancelled objectives are complete and no review result is pending. |
| registered, paused, stopped, completed | archived | No active execution or unresolved result remains. |

`remove` is not a state transition. It removes the registration projection only after archive/retention checks; canonical tombstone and audit evidence remain. It never removes the source repository.

## 3. Objective lifecycle

States: `proposed`, `ready`, `running`, `blocked`, `review`, `completed`, `cancelled`, `superseded`.

- `objective add` creates `ready` for a valid user-authored objective. Planning-agent/import sources create `proposed` until a future acceptance boundary exists.
- `ready -> running` occurs when at least one internal unit is claimed.
- `running -> blocked` requires a durable blocker with next safe action.
- `running -> review` occurs when implementation and independent verification produce a review result.
- `review -> completed` requires the configured integration outcome: explicit apply, explicit accept-without-apply for a no-change objective, or another future policy-backed integration result.
- `ready|running|blocked|review -> cancelled` requires operator/policy authority and safe cancellation/cleanup.
- Supersession records the replacing objective and never rewrites history.

TB-1 deterministically creates one implementation task per objective. Internal verifier and cleanup work are run roles, not user-authored objectives. Later decomposition may create several internal tasks but may not alter objective acceptance or policy.

## 4. Internal task and run lifecycle

Schema-20 task states are retained behind the advanced surface. The migration adds project/objective ownership and maps them to summarized project/objective states.

The existing legal task transitions remain valid except:

- project/calendar affinity replaces task-authored widening;
- route pins become diagnostic overrides and are never required for normal scheduling;
- `failed -> ready` requires a classified retry decision, fresh safety gates, and no uncertain external action;
- terminal/pause transitions atomically enqueue cleanup;
- `review -> completed` requires a recorded review-result disposition rather than implying integration.

Run states are normalized to `preparing`, `running`, `awaiting_approval`, `checkpointing`, `handing_off`, `verifying`, `review`, `succeeded`, `failed`, `cancelled`, `uncertain`, and `cleaning`. `uncertain` is terminal for automatic dispatch and requires reconciliation or explicit operator action.

## 5. Calendar revisions and eligibility

Calendar profiles are append-only revisions. A revision contains name, IANA timezone, exactly seven `W`/`O`/`B` day characters, dated exceptions, source, reason, version, superseded revision, and creation time.

Project affinity is `work`, `non_work`, or `both`:

| System day | Work project | Non-work project | Both project |
| --- | --- | --- | --- |
| W | eligible | ineligible | eligible |
| O | ineligible | eligible | eligible |
| B | eligible | eligible | eligible |

Eligibility output pins calendar revision, project setting revision, local date, day kind/source, evaluated instant, eligible result, reason code, and next eligible instant. A revision re-evaluates queued work immediately. Active work crosses an ineligible boundary by checkpointing and pausing, not abrupt termination.

## 6. Settings and policy revisions

Configuration and policy values use append-only revisions with states `active`, `superseded`, and `rejected`. A proposed invalid revision is rejected before persistence; a policy-denied proposal may create a bounded denial event without becoming a revision.

Resolution order is managed constraint, global, agent/account, project, internal task, run. Denies and most restrictive ceilings win. Every effective field records source scope, revision ID, value type, whether it is managed/delegable, and whether changing it requires service restart, reschedule, reprobe, or run checkpoint.

Repository-projected settings are untrusted proposals. They cannot govern the current run or widen the policy used to import them.

## 7. Approval lifecycle

States: `pending`, `allowed`, `denied`, `expired`, `consumed`, `cancelled`, `invalidated`.

1. A structured agent request is canonicalised with project/run identity, tool, argv/input digest, cwd, file effects, network destinations, secret references, recipients, cost ceiling, sandbox attestation, and expiry.
2. Policy returns `allow`, `deny`, or `ask`.
3. `ask` creates one `pending` approval and one notification-outbox event atomically, keyed by action digest for deduplication.
4. The operator may move `pending -> allowed|denied`. A changed/expired action cannot be decided.
5. Only the execution broker may move `allowed -> consumed`, atomically with authorising the exact action.
6. Pending approvals expire by time, cancel with their run, and invalidate when policy, sandbox, input, cwd, recipient, cost, or action digest changes.

Allowed approval is not a reusable policy rule. A future “always allow” operation must create a separately reviewed policy revision.

## 8. Notification and delivery lifecycle

Canonical event creation and notification intent share the originating transaction. Notification states are `unread` and `acknowledged`. Delivery attempts have separate states `pending`, `delivering`, `delivered`, `retry_wait`, `failed`, and `dead_letter`.

- Delivery failure never rolls back the canonical event or approval.
- Acknowledgement changes inbox state only and grants no authority.
- Deduplication keys group repeated health/quota alerts while preserving event counts and first/latest times.
- Quiet hours delay non-critical desktop attempts. Critical security, emergency-stop, and expiring approval events remain visible in the local inbox immediately.
- Delivery retries are bounded and idempotent; they cannot repeat an approval decision or external agent action.

## 9. Secret metadata lifecycle

Canonical state contains metadata with states `active`, `rotation_due`, `unavailable`, `revoked`, and `deleted`; the value remains exclusively in the selected protected backend.

- `add` creates protected value first, then metadata; failure removes the just-created value where safe or records reconciliation work.
- `test` proves lookup/format/readiness without returning the value or performing paid/provider work.
- `rotate` writes a new backend version, atomically switches metadata, invalidates probes/sessions, and schedules destruction of the old version after a bounded rollback window.
- `remove` first proves there is no active projection/run, then revokes the backend value and tombstones metadata.
- Any projection has project, agent, run, purpose, expiry, and destruction identity. Projection failure denies execution.

For subscription CLIs, the target is a Garnish-owned provider-specific auth broker/profile isolated from coding shells. Host user authentication is not silently copied into containers. Until a concrete adapter proves this property, container eligibility is false and the existing honest fallback boundary remains separate.

## 10. Sandbox and execution-plane lifecycle

States: `requested`, `preparing`, `attesting`, `attested`, `running`, `stopping`, `cleaning`, `cleaned`, `quarantined`, `failed`.

- Only `attested` may become `running` for autonomous writes/builds/tests.
- Attestation pins requested spec hash, inspected runtime state, image digest, user, mounts, network, resources, devices/capabilities, secret projections, process supervision, backend/version, time, and expiry.
- Requested properties are not evidence. Inspection mismatch yields `failed` and denies the run.
- Cancellation moves through `stopping`; descendant termination evidence precedes cleanup.
- Failed cleanup moves to `quarantined` with a notification and never masquerades as cleaned.

Garnish owns the sandbox specification and attestation. AoE may implement session/process lifecycle behind the adapter, but its identifiers/defaults are evidence inputs rather than authority.

## 11. Cleanup lifecycle

Cleanup records are immutable plans plus attempts. States: `pending`, `capturing`, `eligible`, `running`, `complete`, `quarantined`, `failed`, `cancelled`.

The plan identifies every exact Garnish-owned container, worktree, branch, temp path, secret projection, and artifact candidate with ownership proof and retention reason. Capture persists required patch/base/manifest/verification/handoff evidence before deletion eligibility.

- Source checkout, unowned branches/worktrees, unexpected commits, and unresolved paths are never cleanup targets.
- A changed plan digest conflicts instead of broadening deletion.
- Cleanup is idempotent; “already absent” is success only when ownership evidence and source invariants still match.
- Branch cleanup uses Git worktree ownership checks and expected base/commit checks. Unexpected data causes quarantine and notification.

## 12. Handoff lifecycle

States: `drafting`, `ready`, `consumed`, `superseded`, `invalid`.

A ready handoff contains objective/task/run identity, goal, acceptance, source/base/patch state, bounded command/verification results, decisions, assumptions, blockers, artifacts, unverified facts, current quota/policy/calendar revisions, and next safe action. It contains no private chain-of-thought or vendor session secret.

The receiving route validates content hash, source/base state, policy, calendar, quota, capabilities, and sandbox before atomically consuming it. A handoff is single-consumer; additional receivers require a superseding handoff. Failed validation creates `invalid`, preserves evidence, and does not start work.

## 13. Review result and integration lifecycle

States: `pending`, `applied`, `discarded`, `conflicted`, `expired`.

- Verification creates `pending` only after patch, base, changed-path manifest, verification, and source-immutability evidence are durable.
- `project apply` rechecks registered checkout identity, exact base/ancestry, tracked cleanliness, scope, patch hash, policy, and result version. It applies no partial result.
- A mismatch creates `conflicted` and preserves the result for rebase/rework; it does not leave a half-applied checkout.
- `discarded` records actor/reason and releases retention according to policy.
- Apply never commits, pushes, merges, changes branches, or deploys.

