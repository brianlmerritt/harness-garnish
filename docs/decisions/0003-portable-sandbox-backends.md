# ADR 0003: Portable sandbox backends

- Status: accepted
- Date: 2026-07-19

## Decision

Define independent Docker, rootless Podman, and Apple Container adapters. Select a backend only after a health and capability probe. Docker is the first vertical-slice backend; rootless Podman is preferred on Linux when available; Apple Container is a supported macOS option after conformance tests.

A backend may declare `secure_container=true` only when runtime inspection verifies the task worktree is the sole writable project mount, no host/container socket is mounted, credentials are explicitly scoped, the user/resources/network match policy, and cleanup/cancellation are supported.

Backend names do not imply equivalent security. Unsupported controls are capability failures, not warnings that scheduling may ignore.

## Consequences

- Scheduler and policy remain runtime-neutral.
- Platform-specific networking, filesystem, and SELinux behaviour need fixtures.
- AoE may supervise sessions while Garnish owns sandbox construction if AoE cannot meet the effective-profile contract.

