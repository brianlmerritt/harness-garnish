# Harness Garnish

Harness Garnish is a local control plane for quota-aware, policy-controlled AI-assisted software development. It coordinates existing coding-agent CLIs and optional API-backed agents, while keeping scheduling, approvals, canonical state, verification, and recovery outside the agents themselves.

Phase 0 established the architecture under [`docs/`](docs/README.md). The Phase 1 Rust vertical slice is now implemented and locally verified; its exact evidence, limitations, and deferred smoke tests are recorded in [`docs/phase-1-exit-report.md`](docs/phase-1-exit-report.md).

Phase 2 is in progress. Its durable-scheduler design now includes timezone-aware work/off calendars, `W`/`O`/`B` task affinity, dated calendar exceptions, and explicit Linux/WSL2 proof milestones in [`docs/phase-2-plan.md`](docs/phase-2-plan.md).

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
garnish --data-dir .garnish-state scheduler tick --instance local-1 --generation 1 --provider fake --account test --max-active 2
garnish --data-dir .garnish-state scheduler wakes
```

Leadership is fenced by a monotonically increasing generation. Claims, concurrency checks, and project locks commit atomically; a stale leader cannot claim work.

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
garnish --data-dir .garnish-state runtime cancel --run RUN_ID --reason "user requested"
garnish --data-dir .garnish-state runtime retry-state --task TASK_ID
garnish --data-dir .garnish-state runtime retry-limit --task TASK_ID --limit 3
garnish --data-dir .garnish-state runtime circuits
```

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

## Repository authority

The user manages branches and commits for this repository. Agents may edit the current checkout when explicitly asked, but must not create or switch branches, commit, push, open pull requests, merge, or alter remotes.
