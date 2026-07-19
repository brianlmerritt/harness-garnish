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

Calendar gating is external to the agents. An ineligible task remains ready with an explained next wake time; a running task that crosses a day boundary will checkpoint and pause at the next safe boundary once the Phase 2 daemon is connected.

The scheduler arbitration commands are deliberately explicit while the daemon is being built:

```console
garnish --data-dir .garnish-state scheduler register --instance local-1 --hostname my-mac
garnish --data-dir .garnish-state scheduler acquire-leader --instance local-1
garnish --data-dir .garnish-state scheduler tick --instance local-1 --generation 1 --provider fake --account test --max-active 2
garnish --data-dir .garnish-state scheduler wakes
```

Leadership is fenced by a monotonically increasing generation. Claims, concurrency checks, and project locks commit atomically; a stale leader cannot claim work.

## Repository authority

The user manages branches and commits for this repository. Agents may edit the current checkout when explicitly asked, but must not create or switch branches, commit, push, open pull requests, merge, or alter remotes.
