# Policy model

## Objectives

The policy engine decides whether an action is allowed, denied, or requires a human approval. It executes outside the model, evaluates immutable inputs, and records the decision and provenance. Agent preference cannot override policy.

## Configuration layers and precedence

From strongest to weakest:

1. organisation/managed policy;
2. global user policy;
3. named account/agent profile;
4. project policy;
5. task override;
6. run request.

For each field, a higher layer declares whether lower layers may narrow only, choose from an allowlist, or widen within a ceiling. Denies win. Budgets and resource ceilings resolve to the most restrictive applicable value unless explicitly marked additive.

`garnish config explain <path>` must show effective value, source file/revision, overridden values, and whether the field is delegable. Unknown security-relevant keys are errors.

## Effect classes

| Class | Typical effects | Default |
| --- | --- | --- |
| 0 | State reads, planning, status, diff/log inspection | Automatic |
| 1 | Writes and tests within an owned worktree and attested secure container | Automatic when declared scope and policy match |
| 2 | Dependency download, selected secret access, new service, host configuration, SELinux relabel, long/high-cost run | Pre-approved narrow rule or human approval |
| 3 | Branch/commit policy changes, pull/fetch when restricted, push/PR/merge, deployment, DB migration, external message, broad network, credential change, destructive Git, persistent deletion, updater activation | Explicit approval unless a managed/project policy specifically grants the exact effect |

Class is determined by deterministic action metadata. An agent-provided label is ignored.

## Default policy

```yaml
schema_version: 1
autonomy:
  require_secure_container_for_writes: true
  allow_class_0: true
  allow_class_1: true
  class_2: ask
  class_3: ask
sandbox:
  backends: [docker, podman, apple-container]
  network: off
  non_root: required_when_supported
  host_home_mount: deny
  container_socket_mount: deny
  unrelated_project_mount: deny
  max_checkpoint_seconds: 300
git:
  inspect: allow
  create_worktree: allow
  create_branch: allow
  local_commit: allow
  fetch_or_pull: ask
  push: deny
  create_pr: deny
  merge: deny
  change_remote: deny
  destructive: deny
  update_submodule_pointer: ask
api:
  openai: { enabled: false }
  anthropic: { enabled: false }
quota:
  reserve_percent: 20
  unknown_unattended: deny
  max_phase_seconds: 2700
  max_checkpoint_seconds: 300
updates:
  mode: manual
  channel: stable
notifications:
  local: true
  remote: false
```

This is illustrative schema, not committed configuration syntax. The implementation schema must be versioned and validated.

## Project-specific Git policy

Git effects are independently configurable because project workflows vary. A project can require manual branch creation and commits, allow local agent commits but deny remotes, or explicitly permit scoped PR creation.

The effective policy for Harness Garnish itself is stricter than the default:

```yaml
git:
  inspect: allow
  create_worktree: deny
  create_branch: deny
  local_commit: deny
  fetch_or_pull: deny
  push: deny
  create_pr: deny
  merge: deny
  change_remote: deny
  update_submodule_pointer: deny
```

The user performs these operations. This rule applies to development of this repository and must be represented in project configuration when implementation begins.

## Secure-container autonomy

Class 1 automation requires:

- effective policy allowing the action;
- sandbox attestation with `secure_container=true` and unexpired evidence;
- owned worktree/path scope;
- no required interactive approval from the selected adapter;
- fresh-enough quota or an explicit unknown-quota exception;
- a checkpoint and cancellation strategy;
- resource and retry budget.

If any condition fails, the action is denied or enters `awaiting_approval`; the system does not silently lower the security bar. Host-side vendor CLIs may still be used, but then only their declared sandbox protects host execution and the run is not described as fully container-isolated.

## Quota policy

### Surfaces

Policy addresses provider/account surfaces individually. Example:

```yaml
quota:
  default_reserve_percent: 20
  default_max_checkpoint_seconds: 300
  unknown_unattended: deny
  surfaces:
    claude:max-primary:five-hour:
      reserve_percent: 25
    claude:max-primary:weekly:
      reserve_percent: 20
    codex:personal:weekly:
      reserve_percent: 15
    antigravity:free:five-hour:
      reserve_percent: 35
      max_checkpoint_seconds: 120
```

Each surface gate checks effective remaining amount against reserve plus a forecast interval. If any required surface fails, the route is rejected or scheduled after reset. Paid overage is another explicit surface/policy; its existence never implies consent to spend.

### Live override

The user can change remaining percentage, reserve, reset, confidence, or overage permission globally or per project. An override requires reason and optional expiry. It creates an event and triggers rescheduling. It cannot erase the provider observation.

An active phase evaluates the change at the next checkpoint unless the new policy is an emergency denial, revokes a secret/network grant, or requests immediate pause/stop.

## Direct API policy

OpenAI and Anthropic API providers are disabled by default. Enabling one per project requires:

- provider and secret reference;
- hard currency and/or token/request budget and period;
- allowed models and maximum per-request output;
- allowed tools and network/secret grants;
- retry and concurrent-request ceilings;
- whether planner, implementer, verifier, or reviewer roles may use it;
- whether exhausted subscription quota may trigger API routing;
- an approval policy for budget changes.

Budget evaluation uses actual spend where reported and a conservative estimate otherwise. A request is denied when its maximum reservation could exceed remaining budget. Budget changes are Class 2 or 3 according to the monetary impact.

## Network policy

Network is off in the main execution phase by default. Setup may receive a domain/registry allowlist and is recorded as a separate phase. IP-only or wildcard grants require stronger approval. Redirects are revalidated. Localhost access to control, runtime, metadata, or credential services is denied from task containers unless explicitly required through a scoped proxy.

## Secret policy

Secrets use provider references such as OS keychain, supported secret manager, or protected file/environment provider. Policy specifies secret ID, recipient adapter, purpose, phase, delivery method, expiry, and redaction fingerprints. Values are never stored in SQLite, argv, project configuration, events, or patches.

Subscription CLI authentication inside containers is explicit opt-in. A writable global credential directory is denied. Prefer one of:

1. short-lived task-specific token/file projection;
2. minimal read-only copied material with cleanup and documented risk;
3. a host-side authenticated CLI with its own sandbox and a precise isolation statement.

## Skills, MCP, and ACP

- Skills require source, version/hash, trust level, compatible adapters, required tools, and project/task attachment.
- Untrusted repository content cannot auto-install or enable a skill.
- MCP servers require executable/source trust, tool allowlist, network/secret policy, lifecycle timeout, and context limits.
- ACP is a transport capability, not an authority source. ACP approval events are mapped into Garnish approval decisions.
- Core Git, policy, state, quota, sandbox, and verification functions remain deterministic built-ins.

## Approval request schema

Every request shows:

- proposed action and target;
- concrete effect and exact argv/API description, with secrets redacted;
- effect class and risk;
- reversibility/rollback;
- requested scope and duration;
- policy fields that caused the request;
- safer alternatives;
- action digest, expiry, and whether one-shot.

Approval is invalid if target, command, effective policy, base commit, relevant sandbox attestation, or secret/network scope changes. One-shot approval is atomically consumed. A reusable approval is converted into a reviewed policy revision; it is never an unbounded “always allow” token.

## Updates

Manual update is default; user may select automatic stable-channel updates. Policy covers check, download, stage, and activate separately. Activation is Class 3 because it changes the privileged control plane and may migrate state. A narrow managed policy may pre-authorise signed, compatible, rollback-ready idle activation.

Agents and project repositories cannot modify updater trust roots, update policy, or the staged binary. Downgrade is allowed only when database compatibility is proven or a verified pre-migration backup is restored.

## Decision result

The engine returns:

```text
allow | deny | require_approval
decision_id
effective_policy_hash
effect_class
matched rules and provenance
required conditions
expiry/re-evaluation time
```

The caller must present the same action digest when claiming an allow/approval. All decisions are events, including denials.

