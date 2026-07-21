# ADR 0015: Native CLI release archives

- Status: accepted
- Date: 2026-07-21

## Context

The source-run CLI MVP is accepted, but `cargo build` is not an installation contract and a path under `target/` is not a distributable product. Harness Garnish needs a small release boundary that can be exercised without provider traffic, does not imply unsupported cross-compilation, and does not prematurely introduce package repositories, automatic updates, signing infrastructure, or release publication authority.

The executable embeds SQLite through the bundled Rust dependency, but it still targets a specific operating system, architecture, and native runtime. macOS and Linux therefore need independently built and tested artifacts. WSL2 uses the Linux executable contract when the checkout, state, and registered repositories remain in the Linux filesystem.

## Decision

Harness Garnish produces versioned native `.tar.gz` archives from an explicitly supported host Rust target. The initial release targets are:

- `aarch64-apple-darwin`, labelled `macos-aarch64`; and
- `x86_64-unknown-linux-gnu`, labelled `linux-x86_64`, including x86-64 WSL2.

An unsupported host target fails closed rather than emitting a plausibly named artifact. Each archive contains the native `garnish` executable, `README.md`, `LICENSE`, the installation guide, the operator guide, a version record, and a manifest. Archive member ordering, ownership, permissions, and timestamps are normalized. The packaging command also writes a SHA-256 manifest for every Garnish archive in its selected output directory.

Packaging is local and non-publishing. It does not create a Git tag, commit, branch, release, package-repository entry, signature, notarization request, or remote upload. A separate quota-free acceptance script verifies the checksum, extracts the archive into a temporary directory, checks the packaged documentation and executable, initializes private state, registers a temporary Git repository, runs one deterministic fake task, verifies the detached review result, and proves that source HEAD and tracked files remain unchanged while Garnish-managed projections stay confined to `.harness-garnish/`. The fixture disables system/global Git configuration, templates, hooks, signing, automatic line-ending conversion, and filesystem monitors so host preferences cannot silently alter or block package acceptance.

The packaged executable is invoked directly and does not depend on a Rust toolchain at runtime. Git remains required for repository and worktree operations. Codex, bubblewrap, protected provider credentials, and network access are optional lane-specific dependencies and are never bundled.

## Consequences

- A technically competent operator can install a tested binary without placing the repository's Cargo target directory on `PATH`.
- Release acceptance consumes no subscription quota or paid API credit and cannot publish an artifact.
- Linux artifacts inherit the GNU libc compatibility floor of their build host; builds intended for wider distribution must use an explicitly chosen baseline host.
- macOS archives are unsigned and unnotarized. Operators must not present them as Gatekeeper-ready public releases.
- Intel macOS, Linux AArch64, native Windows, Homebrew, Debian/RPM packages, signatures, SBOMs, provenance attestations, automatic updates, and hosted release publication remain later release-engineering work.
- Database migrations remain forward-only for the current CLI. Operators must make and retain a verified state backup before upgrading and must not assume an older binary can reopen a database migrated by a newer version.
