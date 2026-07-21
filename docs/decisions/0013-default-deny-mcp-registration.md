# ADR 0013: Default-deny MCP registration before execution

- Status: accepted
- Date: 2026-07-21

## Context

An MCP server is executable code and its tools, network access, secret access, output, and lifecycle can all expand an agent's authority. Treating a discovered server or advertised tool as trusted would let an extension bypass the control plane. The first controlled-extension slice therefore needs durable trust configuration without prematurely creating a process-execution path.

## Decision

Garnish stores append-only, project-scoped MCP server revisions. A revision contains an exact server name, `stdio` transport, absolute executable path, pinned lowercase SHA-256 digest, argv array, exact tool allowlist, exact network-host grants, protected secret references, startup/request timeouts, context/output byte ceilings, source, reason, and enabled flag. The latest revision for a project/name is authoritative and links to the revision it supersedes.

Registration defaults to disabled. An enabled revision requires at least one exact allowed tool. Wildcard hosts, relative executable paths, inline secret values, shell command strings, unsupported transports, and unbounded lifecycle or content limits fail closed. Events record the digest, allowlisted tool names, grant counts, and limits, but omit argv and secret references.

This slice deliberately provides no MCP process launch, protocol exchange, discovery, installation, secret resolution, or tool-call authorization path. The enabled flag records administrative eligibility only. A later execution ADR must re-hash the executable immediately before launch and prove that core policy, task capability, action-digest approval, sandbox, quota/budget, and independent-verification gates remain authoritative.

## Consequences

- Configuration and normal tests cannot execute third-party code, access a network, resolve credentials, or consume provider quota.
- Server/tool discovery cannot silently broaden authority; every tool grant is an exact durable operator choice.
- Registration is necessary but never sufficient for execution.
- P4-15 remains incomplete until the actual lifecycle and tool-call boundary has machine evidence, alongside remote approval, skill, and ACP boundaries.
