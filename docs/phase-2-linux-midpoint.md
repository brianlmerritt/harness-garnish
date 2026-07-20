# Phase 2 Linux midpoint evidence

## Result

The Linux scheduler/daemon midpoint passed on a user-provided VPS on 2026-07-19. The run used `scripts/test-linux-midpoint` as a dedicated non-root user and consumed no provider quota.

## Captured evidence

- Rust library suite: 40 passed, 0 failed, 1 explicitly ignored real-Docker test.
- CLI suite: 2 passed, 0 failed.
- MVP vertical slice: 1 passed, 0 failed.
- AoE authenticated loopback lifecycle passed.
- Process-group timeout and descendant cleanup passed.
- Bounded daemon run completed two ticks and stopped through `max_ticks`.
- Signal-driven daemon run completed nineteen ticks, handled `TERM`, and reported `shutdown_reason: signal`.
- Scheduler recovery counts were zero on the clean fixture, as expected.
- State directory mode was `0700`; SQLite database mode was `0600`.

## Remaining Linux runtime evidence

Neither Podman nor Docker was installed on the VPS, so the script recorded both runtime probes as skipped. This does not invalidate the scheduler, signal, process-cleanup, filesystem-permission, or restart midpoint. It does mean Harness Garnish does not yet claim Linux container-runtime conformance. Rootless Podman and/or Docker capability and sandbox conformance must be run later on a host where the selected runtime is installed.

The opt-in real-Docker test remained ignored by design because no healthy daemon and digest-pinned fixture image were supplied.

## Podman follow-up

A follow-up run used Linux `6.8.0-107-generic` on `x86_64` with Rust/Cargo `1.97.1`. It exposed a Linux timing race in the quota-free adapter fixture: the fixture exited without consuming Codex's stdin prompt, so the supervisor correctly reported `EPIPE`. The fixture now drains stdin before returning its result.

The corrected quota-free bundle passed on 2026-07-20:

- Rust library suite: 52 passed, 0 failed, 1 explicitly ignored real-Docker test.
- CLI suite: 2 passed, 0 failed.
- MVP vertical slice: 1 passed, 0 failed.
- Runtime-supervision checkpoint, retry, circuit-breaker, cancellation, and descendant-cleanup tests passed.
- Bounded and signal-driven daemon checks passed with the expected `max_ticks` and `signal` shutdown reasons.
- State directory and database modes remained `0700` and `0600`.
- `podman info` succeeded and reported a healthy rootless runtime.
- Docker was not installed, so Docker conformance remains separate and opt-in.

This closes the Linux midpoint and rootless-Podman capability probe. The backend-specific Podman sandbox conformance test is now available through `scripts/test-podman-conformance`, but this evidence remains capability-only until the VPS is rerun with `GARNISH_REAL_PODMAN_IMAGE` set to a local digest-pinned image.
