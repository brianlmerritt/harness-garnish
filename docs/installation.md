# Installation and binary packaging

This guide installs the accepted source-run CLI MVP as an ordinary `garnish` command. It covers macOS Apple Silicon, native x86-64 Linux, and x86-64 WSL2. Native Windows, Intel macOS, Linux AArch64, package repositories, signed/notarized releases, and automatic updates are not currently supported release targets.

## Runtime requirements

Every installation requires:

- Git, because Garnish creates isolated Git worktrees and detached verification worktrees;
- a Linux-native checkout, state directory, and registered repository when running under WSL2; and
- enough local space for the SQLite state, worktrees, verifier worktrees, and bounded evidence.

Rust 1.97 or newer and Cargo are required only when building or installing from source. The packaged executable does not require a Rust toolchain at runtime. SQLite is bundled into the executable.

Optional execution lanes add their own requirements:

- Codex subscription execution requires a supported Codex CLI authenticated independently on each host. Ubuntu/WSL2 should install the distribution `bubblewrap` package.
- OpenAI or Anthropic API execution requires an operator-created protected secret reference and an explicitly configured project budget, price, and request plan.
- The read-only dashboard requires a local browser, or an SSH tunnel to the loopback-only listener when operating a remote host.

## Install from source

Run the following from a trusted source checkout. Cargo builds an optimized executable and installs it in Cargo's configured binary directory, normally `$HOME/.cargo/bin`.

Placeholders: none.

```console
cargo install --path . --locked
garnish --version
garnish --help
```

If `garnish` is not found after a successful Cargo installation, add Cargo's binary directory to the current shell search path. Decide separately whether to make that change persistent in the shell profile.

Placeholders: none.

```console
export PATH="$HOME/.cargo/bin:$PATH"
garnish --version
```

Re-running `cargo install --path . --locked --force` replaces the installed executable with the version built from the current checkout. Back up Garnish state before an upgrade as described below.

## Build a native release archive

The repository packaging command supports only the accepted native host targets. It performs an optimized locked build, normalizes archive metadata, and writes the archive plus `SHA256SUMS` under `target/release-packages/`. It does not publish, tag, sign, notarize, or upload anything.

Placeholders: none.

```console
./scripts/package-release
```

Expected archive names are:

- `harness-garnish-VERSION-macos-aarch64.tar.gz` on Apple Silicon macOS; or
- `harness-garnish-VERSION-linux-x86_64.tar.gz` on x86-64 native Linux and WSL2.

`VERSION` denotes the package version printed by `garnish --version`; it is descriptive here and must not be pasted literally into a filename.

Before distributing an archive, exercise exactly what an installer receives. This test rebuilds and packages the current native target, verifies the SHA-256 manifest, extracts into a temporary directory, and runs a complete quota-free fixture task through review.

Placeholders: none.

```console
./scripts/test-release-package
```

The test consumes no provider request, Codex subscription task, or paid API credit. It does not install into the user's real binary directory or modify real Garnish state.

## Install a native archive

Obtain the archive and its `SHA256SUMS` from the same trusted release source. Verify the checksum before extraction.

Placeholders: `ARCHIVE_NAME` must be replaced with the exact archive filename received from the trusted release source.

```console
grep "  ARCHIVE_NAME$" SHA256SUMS > ARCHIVE_NAME.sha256
shasum -a 256 -c ARCHIVE_NAME.sha256
tar -xzf ARCHIVE_NAME
```

Linux systems may use `sha256sum -c ARCHIVE_NAME.sha256` instead of `shasum -a 256 -c ARCHIVE_NAME.sha256`. Verification must report the selected archive as `OK`. Do not install an archive whose filename is absent from the manifest or whose checksum fails. The selected checksum file is derived from the trusted complete manifest so downloading one platform archive does not require downloading every other platform archive named there.

Copy the extracted executable into a user-owned binary directory. This avoids requiring root privileges.

Placeholders: `EXTRACTED_PACKAGE_DIRECTORY` must be replaced with the exact top-level directory created by extracting the verified archive.

```console
install -d "$HOME/.local/bin"
install -m 0755 "EXTRACTED_PACKAGE_DIRECTORY/bin/garnish" "$HOME/.local/bin/garnish"
export PATH="$HOME/.local/bin:$PATH"
garnish --version
garnish --help
```

The archive contains its matching README, license, installation guide, and operator guide. It does not install or alter shell profiles automatically.

## State location and first initialization

Without `--data-dir` or `GARNISH_DATA_DIR`, Garnish uses the operating system's application-data location and reports the exact path in the `init` JSON. For predictable administration across hosts, the operator guide uses an explicit private directory.

Placeholders: none.

```console
export GARNISH_DATA_DIR="$HOME/.local/share/harness-garnish"
garnish init
garnish doctor
```

On Unix, Garnish enforces mode `0700` for its data directory and `0600` for the SQLite database and backups. Do not put the data directory in a Git repository, a shared folder, `/mnt/c` under WSL2, or a directory synchronized to another machine while Garnish is running.

The command emits JSON on stdout for success and bounded JSON on stderr for failure. Exit code `0` is success, `1` is a rejected or failed operation, and Clap uses exit code `2` for invalid command syntax.

## Upgrade and rollback boundary

Create and retain a verified backup before replacing the executable.

Placeholders: none.

```console
garnish ops backup
garnish ops status
```

Stop any running scheduler daemon or dashboard cleanly before replacement. Install the new binary using the same source or archive method, then run:

Placeholders: none.

```console
garnish --version
garnish doctor
garnish ops status
```

Opening state may apply a forward schema migration and creates a verified pre-migration database backup. The current MVP does not provide an automatic updater or a general down-migration. Do not assume that an older executable can reopen state migrated by a newer version. Preserve the pre-upgrade executable and backup until the new version has been accepted.

## Uninstall

For an installation made by Cargo:

Placeholders: none.

```console
cargo uninstall harness-garnish
```

For a user-local archive installation, remove only the installed executable from the known path:

Placeholders: none.

```console
rm "$HOME/.local/bin/garnish"
```

Neither command removes Garnish state, registered repositories, worktrees, receipts, or backups. Review and archive the state separately before any manual deletion. The project deliberately provides no recursive state-deletion command.

## Packaging limitations

- Archives are built natively and are operating-system and architecture specific.
- The Linux archive has the GNU libc compatibility floor of its build host. Use a deliberate older supported build baseline before claiming wide Linux distribution.
- macOS archives are not signed or notarized.
- `SHA256SUMS` proves content identity against a trusted manifest; it does not establish who produced the manifest.
- The packaging process produces no SBOM, provenance attestation, package-manager metadata, or hosted release.

These limitations do not affect local source installation, but they must be resolved before presenting the archive as a broadly distributed production release.
