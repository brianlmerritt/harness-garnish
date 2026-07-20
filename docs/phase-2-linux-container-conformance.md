# Phase 2 Linux container conformance evidence

## Result

The rootless Podman and Docker sandbox conformance tests passed on the user-provided Ubuntu VPS on 2026-07-20. Both used the locally stored Alpine image pinned to `docker.io/library/alpine@sha256:28bd5fe8b56d1bd048e5babf5b10710ebe0bae67db86916198a6eec434943f8b`. The tests performed no image pull and no container-network access during execution.

## Host and runtimes

- Ubuntu 24.04.4 LTS, Linux `6.8.0-107-generic`, `x86_64`.
- Rootless Podman `4.9.3`.
- Docker client and server `29.6.2`.
- Docker used `overlayfs` with the containerd snapshotter, systemd cgroups v2, and runc `1.3.6`.
- Docker daemon access was configured for the dedicated non-root `blm` user through the existing `docker` group; the SSH session was restarted before testing.

## Podman result

`scripts/test-podman-conformance` passed its single explicitly selected real-runtime test:

- the digest-pinned local image was accepted with pulls disabled;
- the rootless/local runtime and effective UID/GID mapping were attested;
- Podman 4.9's null pre-start capability slices were verified together with bounding and added-capability state;
- create, pre-start inspection, attached execution, worktree output, and cleanup passed;
- result: 1 passed, 0 failed.

## Docker result

`scripts/test-docker-conformance` passed its single explicitly selected real-runtime test:

- the digest-pinned local image was accepted with pulls disabled;
- network, capability, privilege, namespace, resource, image, user, mount, proxy-environment, and tmpfs state were attested before start;
- create, pre-start inspection, attached execution, worktree output, and cleanup passed;
- result: 1 passed, 0 failed.

## Scope

This supplies real Linux conformance evidence for the Podman and Docker sandbox backends. It does not supply Apple Container evidence, which requires a compatible macOS host, and it does not enable real coding-agent or provider calls.
