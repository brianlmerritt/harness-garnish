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
