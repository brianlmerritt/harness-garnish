# Harness Garnish

Harness Garnish is a local control plane for quota-aware, policy-controlled AI-assisted software development. It coordinates existing coding-agent CLIs and optional API-backed agents, while keeping scheduling, approvals, canonical state, verification, and recovery outside the agents themselves.

Phase 0 established the architecture under [`docs/`](docs/README.md). The Phase 1 Rust vertical slice is now implemented and locally verified; its exact evidence, limitations, and deferred smoke tests are recorded in [`docs/phase-1-exit-report.md`](docs/phase-1-exit-report.md).

Phase 2 is complete. Its schema-7 durable scheduler, supervision/recovery matrices, day-aware scheduling, container evidence, and final macOS/Linux/WSL2 results are recorded in [`docs/phase-2-exit-report.md`](docs/phase-2-exit-report.md).

Phase 3 is complete. Its schema-14 multi-agent capability routing, quota observations/reservations/forecasting, authenticated local operator interface, separate verifier runs, and final macOS/Linux/WSL2 results are recorded in [`docs/phase-3-exit-report.md`](docs/phase-3-exit-report.md).

Phase 4 is in progress. Its network-free first slice adds project-scoped API budgets and atomic paid-usage reservations before any real OpenAI or Anthropic request is enabled; see [`docs/phase-4-plan.md`](docs/phase-4-plan.md).

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

## Phase 1 quick start

Rust 1.97 or newer and Git are required. All normal tests use fake agents and consume no provider quota or API budget.

```console
cargo build --locked
cargo test --workspace
cargo run -- --data-dir /tmp/garnish-state doctor
```

The CLI emits JSON on stdout for success and JSON on stderr for failure. Exit code `0` means the command completed; exit code `1` means validation, policy, quota, adapter, runtime, or state handling rejected or failed the command. Argument syntax errors are emitted by Clap with exit code `2`.

Typical local flow:

```console
garnish --data-dir .garnish-state init
garnish --data-dir .garnish-state project add --slug example --title Example --path /absolute/path/to/repository
garnish --data-dir .garnish-state quota set --provider fake --account test --surface five_hour --remaining-percent 90
garnish --data-dir .garnish-state task add --project example --title "Bounded change" --goal "Create result.txt" --accept "result.txt contains done" --verify-argv '["grep","-q","done","result.txt"]' --scope result.txt --non-scope "remote Git" --fake-write-path result.txt --fake-write-content done
```

`task run` is deliberately limited to deterministic fake adapters in the normal path. Real Codex, Claude Code, Antigravity, AoE, and runtime smoke tests remain individually opt-in and are never part of default CI.

### Day-aware scheduling

Weekly patterns are Monday through Sunday. `W` means a user workday and `O` an off day; the default is `WWWWWOO`. Tasks use `--day-affinity W`, `O`, or `B` (both, the default).

```console
garnish --data-dir .garnish-state schedule configure --slug uk-week --timezone Europe/London --weekly-pattern WWWWWOO
garnish --data-dir .garnish-state schedule assign --project example --calendar uk-week
garnish --data-dir .garnish-state schedule exception --calendar uk-week --date 2026-12-25 --kind O --reason "holiday"
garnish --data-dir .garnish-state schedule evaluate --task TASK_ID
garnish --data-dir .garnish-state schedule preview --provider fake --account test
```

Calendar gating is external to the agents. An ineligible task remains ready with an explained next wake time; a running task that crosses a day boundary requests a checkpoint and graceful pause at the next safe boundary.

The scheduler arbitration commands are deliberately explicit while the daemon is being built:

```console
garnish --data-dir .garnish-state scheduler register --instance local-1 --hostname my-mac
garnish --data-dir .garnish-state scheduler acquire-leader --instance local-1
garnish --data-dir .garnish-state scheduler tick --instance local-1 --generation 1 --provider fake --account test --max-active 2 --max-active-per-adapter 1 --max-active-per-account 1
garnish --data-dir .garnish-state scheduler wakes
```

Leadership is fenced by a monotonically increasing generation. Claims, global/adapter/account concurrency checks, and project locks commit atomically; a stale leader cannot claim work. Scheduler exclusions are persisted with stable machine reason codes.

Projects can be paused independently, and task admission can include an RFC 3339 deadline and repeatable adapter capability requirements:

```console
garnish --data-dir .garnish-state project pause --project example --reason "project maintenance"
garnish --data-dir .garnish-state project resume --project example --reason "maintenance complete"
garnish --data-dir .garnish-state task add --project example --title "Timed work" --goal "Create result.txt" --accept "result.txt contains done" --verify-argv '["grep","-q","done","result.txt"]' --scope result.txt --non-scope "remote Git" --deadline-at 2026-07-21T17:00:00Z --requires-capability structured_output --fake-write-path result.txt --fake-write-content done
```

The continuous daemon owns and renews both the leader lease and its task claims. `TERM` or `INT` requests a bounded graceful stop that releases its claims and returns not-yet-started tasks to `ready`:

```console
garnish --data-dir .garnish-state scheduler daemon --instance local-1 --hostname my-mac
```

By default the daemon only arbitrates and holds eligible work. `--execute-fake` additionally exercises the quota-free fake claim-to-run path: the route, claim, single-use action key, run, run lease, and project lock are bound durably before fake execution begins. Real agents remain opt-in while their secure-container execution path is connected. `--max-ticks` provides a bounded diagnostic run.

```console
garnish --data-dir .garnish-state scheduler daemon --instance local-1 --hostname my-mac --execute-fake --max-ticks 1
```

Runtime supervision persists lease-fenced checkpoint decisions, cancellation intent, process termination evidence, retry budgets/backoff, and adapter circuit state. Pause/cancel decisions retain the run lease until the worker records TERM/KILL completion.

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

```console
garnish --data-dir .garnish-state agent refresh --valid-seconds 300
garnish --data-dir .garnish-state agent status
```

A missing or unsupported CLI is recorded as evidence rather than treated as healthy. The pure multi-candidate kernel hard-filters probe freshness, health, capabilities, quota headroom, and exact manual pins before applying a deterministic recorded score.

Schema 9 makes task pins durable and lets scheduler ticks/daemons consider multiple route identities. Pin changes always require a reason:

```console
garnish --data-dir .garnish-state task pin TASK_ID --adapter codex --provider openai --account primary --reason "preserve session continuity"
garnish --data-dir .garnish-state task unpin TASK_ID --reason "continuity no longer required"
garnish --data-dir .garnish-state scheduler tick --instance local-1 --generation 1 --candidate codex:openai:primary --candidate claude:anthropic:max
```

`TASK_ID` is a placeholder for the ID returned by `task add`. Candidate values are real configured identities, not fallback API permissions. A pin cannot bypass capability, health, policy, quota, or concurrency gates.

Schema 14 includes the current CodexBar usage JSON contract, append-only quota observations and collection attempts, five-minute freshness by default, atomic scheduler reservations, durable historical-usage samples, and separately attributable verifier runs. A real refresh may read provider authentication state and access the network, but it does not submit an agent task. It is always an explicit command:

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

```console
garnish --data-dir .garnish-state quota forecast --adapter codex --provider codex --account default --estimated-seconds 600 --uncertainty-percent 25
```

`quota record-usage` is currently a collector/operator ingestion contract, not an instruction to estimate percentages by hand. Its required `--evidence-id` must be a stable real run or collector identifier; `--source` must name the real evidence source. Provider-reported, collector-measured, and explicit user-reported confidence are distinct from untrusted agent output.

### Phase 4 API budget control plane

Schema 16 keeps paid OpenAI/Anthropic API budgets separate from subscription quotas and adds append-only model-price evidence with exact categorized-token costing. It has no real HTTP transport: configuration, fixture execution, and these read commands cannot make a provider request or spend credit.

```console
cargo run --locked -- --data-dir .garnish-state api budget-status
cargo run --locked -- --data-dir .garnish-state api reservations
cargo run --locked -- --data-dir .garnish-state api spend
cargo run --locked -- --data-dir .garnish-state api price-status
cargo run --locked -- --data-dir .garnish-state api price-set --help
```

These five commands contain no placeholders; the last prints the required pricing-evidence fields without changing state. A project budget can be configured through `api budget-set`, but configuration alone never enables spending: effective policy is independently default-deny, and no subscription-quota condition can select paid API use. Price rates are explicit integer currency micros per million tokens and are never fetched or guessed. Secret fields accept only a locator shaped as `env:NAME`, `keychain:SERVICE/ACCOUNT`, or `file:/absolute/path`; they never accept a key value. The authenticated read-only dashboard shows API budgets, outstanding reservations, and settled usage under **Agents & quotas**.

Protected secret resolution is host-side and bounded. On Unix, `file:` targets must be regular files owned by the Garnish user with no group/other permissions (normally mode `0600`); symlinks are rejected. `keychain:` currently requires macOS. WSL2 and native Linux use `env:` or a protected Linux-side file. No API transport is enabled yet, so users should not configure or test a live credential at this stage.

Successful fake execution now creates separate implementer and verifier run records. The quota-free `garnish-command-verifier:local:default` is independently selected, receives a clean detached verification worktree and its own evidence directory, and runs only the task's predeclared verification argv. It is a deterministic command verifier, not a claim of semantic agent review. Default policy requires a different verifier adapter; project policy can also require a different provider.

### Local operator interface

The initial human-facing interface is an authenticated, read-only dashboard over canonical Garnish state:

```console
cargo run --locked -- --data-dir .garnish-state ui serve --port 7467
```

Open the exact one-time `url` printed by the command. There are no placeholders in this command. The server binds only to `127.0.0.1`, exchanges the random startup token for a strict local cookie, and displays Overview, Projects, Queue explanations, Agents & quotas, Approvals, Activity, and Settings. Stop it with `Ctrl-C`.

The URL is sensitive while that UI process is running; do not paste it into project files or shared logs. This first slice cannot modify state. Pause, resume, approval decisions, quota overrides, and emergency controls remain available through the CLI until the web mutation contract has CSRF protection, explicit confirmations, bounded typed inputs, policy checks, and durable evidence.

Operational controls are durable and emit bounded JSON. `pause` stops new claims; `emergency-stop` also releases unstarted claims and requests graceful cancellation of active runs. Neither command claims a process has stopped until its supervisor records termination evidence.

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

```console
./scripts/test-linux-midpoint
```

It runs formatting, lint, build and tests; exercises bounded and signal-driven daemon shutdown; verifies private state permissions; and reports rootless Podman and Docker health when installed. The 2026-07-20 VPS run passed this checkpoint and confirmed a healthy rootless Podman capability probe.

Real rootless-Podman 4.9.3 and Docker 29.6.2 conformance subsequently passed on that Ubuntu VPS. The captured runtime and attestation evidence is in [`docs/phase-2-linux-container-conformance.md`](docs/phase-2-linux-container-conformance.md).

The hardened Podman sandbox lifecycle is a separate, explicit opt-in because it needs a real local image. These commands pull Alpine once, derive its real digest automatically, and then run with further pulls and container networking disabled:

```console
podman pull docker.io/library/alpine:latest
PODMAN_IMAGE="$(podman image inspect docker.io/library/alpine:latest --format '{{index .RepoDigests 0}}')"
GARNISH_REAL_PODMAN_IMAGE="$PODMAN_IMAGE" ./scripts/test-podman-conformance
```

The same environment variable adds this conformance test to `test-linux-midpoint` and therefore to the WSL2 bundle. Without it, those bundles report the real-container test as skipped. No agent subscription or API is used.

Docker has an equivalent opt-in test. These commands pull Alpine into Docker's separate local image store, derive the real digest, and run it:

```console
docker pull docker.io/library/alpine:latest
DOCKER_IMAGE="$(docker image inspect docker.io/library/alpine:latest --format '{{index .RepoDigests 0}}')"
GARNISH_REAL_DOCKER_IMAGE="$DOCKER_IMAGE" ./scripts/test-docker-conformance
```

Setting both environment variables when running `scripts/test-linux-midpoint` runs both real backends. Docker and rootless Podman retain separate local image stores, container namespaces, and lifecycle tests.

### WSL2 exit bundle

Use a current rustup-managed Rust toolchain and keep the checkout in the WSL2 Linux filesystem, not under `/mnt/c` or another Windows drive mount. The project requires Rust 1.97 or newer; Rust edition 2024 is supported normally on WSL2 when the compiler is current.

```console
./scripts/test-wsl2-exit
```

The bundle consumes no provider quota. It runs the Linux midpoint, verifies default denial of Windows-mounted project roots, requires a healthy rootless Podman or Docker runtime, exercises operational backup/status, and checks private Linux permissions. It passed on Ubuntu 24.04 under WSL2 with rootless Podman on 2026-07-20; the captured evidence and cgroup fallback caveat are in [`docs/phase-2-wsl2-exit.md`](docs/phase-2-wsl2-exit.md).

### Phase 4 portability checkpoint

After updating the checkout on either the Linux VPS or WSL2, run this exact command as the existing non-root development user:

```console
./scripts/test-phase4-portability
```

There are no placeholders and no additional packages or container images are required. The script removes OpenAI and Anthropic credential variables from the test environment, then runs formatting, strict lint, and the complete fixture-only suite. It automatically distinguishes native Linux from WSL2 and rejects a WSL checkout under `/mnt/<drive>`. Both platforms passed on 2026-07-20; the combined three-platform evidence is in [`docs/phase-4-portability-checkpoint.md`](docs/phase-4-portability-checkpoint.md).

## Repository authority

The user manages branches and commits for this repository. Agents may edit the current checkout when explicitly asked, but must not create or switch branches, commit, push, open pull requests, merge, or alter remotes.
