# Phase 4 portability checkpoint

- Date: 2026-07-20
- Evidence boundary: Git commit `5627fb0` (`phase 4 pt4`)
- Scope: Schema 16 API budget/pricing control plane, categorized provider usage, fake-only execution lifecycle, and protected secret references
- Result: passed on macOS, native Linux, and WSL2

## Safety conditions

The portability script removed `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, and `ANTHROPIC_AUTH_TOKEN` from the test process environment. The suite used fixture responses and fake executables only. It made no provider request and consumed no subscription quota or paid API credit. The two real-container tests remained explicitly ignored because this checkpoint did not opt in with digest-pinned image variables.

## macOS

- Kernel: Darwin 25.5.0 arm64
- Rust: 1.97.1
- Cargo: 1.97.1
- Strict Clippy: passed
- Tests: 129 passed, 0 failed, 2 explicitly ignored

The macOS suite was run directly because `scripts/test-phase4-portability` deliberately accepts only Linux and WSL2. This avoids presenting a Linux portability check as a macOS test while preserving the same format, lint, and fixture-only test gates.

## Linux VPS

- Kernel: Linux 6.8.0-107-generic x86_64 GNU/Linux
- Rust: 1.97.1
- Cargo: 1.97.1
- Script: `./scripts/test-phase4-portability`
- Strict Clippy: passed
- Tests: 129 passed, 0 failed, 2 explicitly ignored
- Terminal evidence: `Phase 4 quota-free portability checkpoint passed on linux`

## WSL2

- Platform detection: WSL2
- Kernel: Linux 6.6.87.2-microsoft-standard-WSL2 x86_64 GNU/Linux
- Rust: 1.97.1
- Cargo: 1.97.1
- Script: `./scripts/test-phase4-portability`
- Strict Clippy: passed
- Tests: 129 passed, 0 failed, 2 explicitly ignored
- Terminal evidence: `Phase 4 quota-free portability checkpoint passed on wsl2`

The script's terminal success is emitted only after format, strict Clippy, and the complete locked workspace test suite succeed with provider credential variables removed.

## Conclusion

P4-14 is established for the Schema 16 intermediate scope: the normal macOS/Linux/WSL2 suites are fixture-only and quota-free. This is not Phase 4 exit evidence. Real provider transports, routing integration, controlled extensions, and any explicitly opted-in paid smoke test remain incomplete.
