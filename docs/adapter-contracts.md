# Adapter contracts

## Common rules

Every adapter has a stable `kind`, semantic contract version, implementation version, configuration schema, supported host platforms, and compatibility range. Probes are side-effect-free unless the user explicitly requests a smoke test.

Adapters must:

- build argv arrays and pass prompts through stdin or files where supported;
- never interpolate untrusted values into a shell command;
- return typed outcomes and stable failure categories;
- declare unsupported capabilities rather than emulate them silently;
- redact at ingestion and presentation;
- support cancellation or declare it absent;
- preserve raw bounded evidence plus parser/schema version;
- expose health and version drift through `garnish doctor`;
- avoid credential discovery beyond configured references and documented user-approved locations.

These are conceptual Rust contracts. Exact crate/API shapes may evolve without changing their semantics.

## Shared types

```rust
struct ProbeResult {
    adapter_key: String,
    executable: Option<PathBuf>,
    version: Option<Version>,
    health: Health,
    capabilities: CapabilitySet,
    evidence: ArtifactRef,
    probed_at: DateTime<Utc>,
    valid_until: DateTime<Utc>,
    failure: Option<Failure>,
}

struct OperationContext {
    task_id: TaskId,
    run_id: RunId,
    worktree: CanonicalPath,
    policy_hash: Digest,
    cancellation: CancellationToken,
    deadline: DateTime<Utc>,
    output_limit: ByteCount,
}
```

Capabilities are namespaced values with parameters, for example `agent.structured_output=jsonl@1`, `session.resume=same-profile`, `sandbox.network=none`, and `process.signal=term-kill`.

## Execution plane

```rust
trait ExecutionPlane {
    async fn probe(&self) -> ProbeResult;
    async fn create(&self, ctx: &OperationContext, spec: SessionSpec) -> Result<SessionRef>;
    async fn start(&self, session: &SessionRef) -> Result<()>;
    async fn send(&self, session: &SessionRef, input: AgentInput) -> Result<()>;
    async fn events(&self, session: &SessionRef, cursor: EventCursor) -> Result<EventStream>;
    async fn status(&self, session: &SessionRef) -> Result<SessionStatus>;
    async fn checkpoint(&self, session: &SessionRef, request: CheckpointRequest) -> Result<CheckpointEvidence>;
    async fn cancel(&self, session: &SessionRef, grace: Duration) -> Result<CancellationEvidence>;
    async fn attach(&self, session: &SessionRef) -> Result<AttachDescriptor>;
    async fn cleanup(&self, session: &SessionRef, retention: RetentionPolicy) -> Result<CleanupEvidence>;
    async fn recover(&self, external_ref: &str) -> Result<RecoveryState>;
}
```

`create` is idempotent by run ID and must not launch work. `start` may be retried safely. Cancellation first requests graceful stop, then bounded escalation, and records descendant cleanup evidence.

### AoE adapter

Initial supported surface:

- probe `aoe --version` and documented help/API health;
- require authenticated loopback `aoe serve`; never use `--no-auth`;
- create a session/worktree through the smallest documented endpoint or CLI;
- use ACP typed events when compatible, otherwise status plus bounded terminal capture;
- serialise sends per AoE session;
- map AoE IDs into external references;
- independently verify worktree and sandbox state;
- pin HTTP/plugin API schema and supported AoE semver;
- treat terminal-output stability only as a hint, not completion.

The adapter must not rely on AoE's database schema. A future Agent Deck or internal plane implements the same contract without changing task state.

## Agent adapter

```rust
trait AgentAdapter {
    async fn probe(&self) -> ProbeResult;
    fn capabilities(&self, probe: &ProbeResult) -> AgentCapabilities;
    fn build_invocation(&self, run: &RunSpec, policy: &EffectivePolicy) -> Result<Invocation>;
    fn parse_event(&self, bytes: &[u8], parser_version: &str) -> ParseOutcome;
    fn classify_exit(&self, process: ProcessExit, evidence: &RunEvidence) -> AgentOutcome;
    async fn checkpoint_hint(&self, session: &SessionRef, handoff_path: &Path) -> Result<()>;
    fn resume_invocation(&self, prior: &NativeSessionRef, run: &RunSpec) -> Result<Option<Invocation>>;
}
```

`Invocation` contains executable realpath, argv, stdin source, cwd, environment allowlist, required secret references, output protocol, timeout, and sandbox/permission expectations. It never contains a shell string.

### Verified initial matrix

| Adapter | Locally probed version | Preferred mode | Structured evidence | Resume | Policy notes |
| --- | --- | --- | --- | --- | --- |
| Codex | 0.144.5 | `codex exec` with prompt via stdin | `--json` JSONL; optional output schema and last-message file | `exec resume` | Select explicit sandbox; unsafe bypass only after external sandbox attestation and explicit policy |
| Claude Code | 2.1.215 | `claude -p` | `--output-format stream-json`, optional JSON schema and hook events | session ID / continue / fork | Explicit allowed/disallowed tools and permission mode; max API budget applies only to API-billed calls |
| Antigravity | 1.1.4 | `agy --print` | text/raw log in current verified interface | conversation ID / continue | Five-minute default print timeout; `--sandbox` and mode are version-specific; unsafe bypass remains externally gated |
| Fake | test fixture | deterministic script | versioned fixture JSONL | deterministic | consumes no provider quota |

The executable probe is authoritative for the installed version. Documentation informs expectations but cannot override the probe. Exact argv templates belong in compatibility fixtures, not this architecture document.

### Failure categories

At minimum: executable missing, unsupported version, authentication required, quota/rate limited, permission required, policy denied, malformed output, parser drift, timeout, cancelled, process crash, sandbox failure, network denied, context limit, provider unavailable, and agent-reported failure. Unknown output remains available as quarantined/raw evidence.

## Sandbox backend

```rust
trait SandboxBackend {
    async fn probe(&self) -> ProbeResult;
    async fn prepare(&self, ctx: &OperationContext, spec: SandboxSpec) -> Result<SandboxRef>;
    async fn inspect(&self, sandbox: &SandboxRef) -> Result<InspectedSandbox>;
    async fn exec(&self, sandbox: &SandboxRef, process: ProcessSpec) -> Result<ProcessHandle>;
    async fn copy_out(&self, sandbox: &SandboxRef, artifact: ScopedArtifactPath) -> Result<ArtifactRef>;
    async fn stop(&self, sandbox: &SandboxRef, grace: Duration) -> Result<()>;
    async fn cleanup(&self, sandbox: &SandboxRef) -> Result<CleanupEvidence>;
    async fn list_orphans(&self, namespace: &str) -> Result<Vec<Orphan>>;
}
```

`SandboxSpec` includes pinned image/digest, non-root identity, exact mounts, environment names, secret references/delivery, network policy, DNS/domain rules, CPU/memory/PID/disk/output/time limits, capabilities/devices/security options, cache namespaces, working directory, labels, and cleanup/retention.

`prepare` labels every external resource with run ID and spec hash. `inspect` compares effective runtime state with the spec and issues a structured attestation. Docker, Podman, and Apple Container conformance suites share behaviour tests but keep backend-specific assertions.

The Phase 2 rootless-Podman implementation creates without pulling, disables networking and inherited proxy variables, uses `keep-id`, drops all capabilities, enables `no-new-privileges`, applies a read-only root and hardened `/tmp`, bounds CPU/memory/PIDs, and mounts only the project worktree. It accepts the sandbox only after inspecting the effective runtime state, including the rootless/local runtime properties and capability sets. Podman 4.9 serializes empty pre-start `EffectiveCaps` and `BoundingCaps` slices as `null`; Garnish verifies both fields together with `CapAdd`, and uses every capability set in the generated OCI configuration only when those inspect fields are unavailable. Missing or malformed evidence fails closed, and failed attestation is cleaned up without starting workload code. `scripts/test-podman-conformance` and `scripts/test-docker-conformance` exercise create, inspect, attached execution, worktree output, and cleanup against explicitly supplied local digest-pinned images.

## Quota provider

```rust
trait QuotaProvider {
    async fn probe(&self) -> ProbeResult;
    async fn accounts(&self) -> Result<Vec<QuotaAccountDescriptor>>;
    async fn snapshot(&self, account: &QuotaAccountRef) -> Result<Vec<QuotaObservation>>;
}

struct QuotaObservation {
    surface_key: String,
    kind: QuotaKind,
    window: QuotaWindow,
    unit: Unit,
    used: Option<Decimal>,
    remaining: Option<Decimal>,
    limit: Option<Decimal>,
    remaining_percent: Option<Decimal>,
    reset_at: Option<DateTime<Utc>>,
    source: ObservationSource,
    observed_at: DateTime<Utc>,
    confidence: Confidence,
    unknown_reason: Option<String>,
}
```

Providers return all visible surfaces independently. Parsing local token logs is `observed_usage`, not `provider_remaining`, unless the vendor report actually supplies the latter.

Initial providers:

- fake deterministic snapshots for tests;
- CodexBar machine-readable usage on supported hosts;
- Tokscale history and secondary subscription observations;
- direct adapter-emitted run usage;
- manual/user override provider as a separate provenance layer.

A no-change result and `unknown` are normal outcomes. Parser drift invalidates freshness and raises a diagnostic; it does not crash the daemon.

## Router and forecast contract

The routing engine is core code, not an adapter. A forecast provider may estimate duration and surface consumption as distributions/ranges. The decision record includes:

- task and capability requirements;
- candidates and hard-filter reasons;
- every relevant quota snapshot/override/reservation;
- expected low/high duration and consumption;
- reserve and reset comparisons;
- score components and policy version;
- chosen candidate or next eligible wake time.

An LLM planner may propose estimates or decomposition, but cannot bypass hard filters.

## Direct API agent provider

```rust
trait ApiAgentProvider {
    async fn probe(&self) -> ProbeResult;
    async fn models(&self) -> Result<Vec<ModelCapability>>;
    async fn reserve(&self, project: ProjectId, request: &ApiRunSpec) -> Result<BudgetReservation>;
    async fn run(&self, ctx: &OperationContext, request: ApiRunSpec) -> Result<ApiEventStream>;
    async fn cancel(&self, provider_request: &ProviderRequestRef) -> Result<()>;
    async fn usage(&self, provider_request: &ProviderRequestRef) -> Result<ApiUsage>;
}
```

OpenAI uses the current Responses API or maintained Agents SDK where it preserves tools, structured output, cancellation, and traces. Anthropic uses its maintained API/agent SDK equivalent. Model IDs and capabilities are discovered/configured data. The provider enforces project enablement, model/tool allowlists, per-request maximums, outstanding reservations, total budgets, retry limits, and rate limits before network access.

API secrets come from a configured secret provider. No adapter may fall back from a subscription CLI to paid API merely because CLI quota is low.

## Verification adapter

Verification commands use argv arrays, a clean sandbox, and the produced commit/patch. The contract returns command, cwd, environment fingerprint, tool versions, start/end, exit status, bounded output artifact, and result. Independent review is a separate role/run and cannot mutate the implementation worktree.

## Projection adapter

The projector renders canonical state to bounded files and returns database version/content hash. Import supports only schema-declared fields and compare-and-swap against the last generated hash. Conflict output is machine-readable and does not partially apply.

## Secret provider

```rust
trait SecretProvider {
    async fn describe(&self, reference: &SecretRef) -> Result<SecretMetadata>;
    async fn project(&self, reference: &SecretRef, grant: SecretGrant) -> Result<SecretLease>;
    async fn revoke(&self, lease: &SecretLease) -> Result<()>;
}
```

The core never requests list-all/read-all access. Secret metadata is non-sensitive. Projection favours a protected file/descriptor over environment and never argv. Redaction fingerprints are derived without storing the value.

## Notification and remote approval

Local notification adapters receive a bounded summary and opaque task/run ID, not secrets or full logs. Remote approval is absent from the MVP. A future adapter requires mutually authenticated SSH/Tailscale transport, actor identity, action-digest binding, expiry, replay prevention, and an authenticated loopback control API behind the tunnel.

## Update provider

```rust
trait UpdateProvider {
    async fn check(&self, channel: &str) -> Result<ReleaseMetadata>;
    async fn stage(&self, release: &ReleaseMetadata) -> Result<StagedRelease>;
    async fn verify(&self, staged: &StagedRelease) -> Result<VerificationEvidence>;
    async fn activate(&self, staged: &StagedRelease, backup: &BackupRef) -> Result<ActivationResult>;
    async fn rollback(&self, activation: &ActivationResult) -> Result<()>;
}
```

Release metadata and artifacts require signature/hash verification. Activation holds updater/migration locks, respects idle/checkpoint policy, performs backup and health checks, and records rollback evidence. Agent code cannot invoke it without the external policy gate.

## Conformance testing

Every adapter ships:

- version/help/output fixtures for supported and unsupported versions;
- deterministic fake executable/server fixtures;
- parser fuzz/property tests and bounded malformed input;
- timeout/cancellation/child cleanup tests;
- secret redaction and path/argv injection tests;
- capability degradation tests;
- no-real-credential CI tests;
- opt-in, labelled real smoke tests that disclose quota/cost impact.
