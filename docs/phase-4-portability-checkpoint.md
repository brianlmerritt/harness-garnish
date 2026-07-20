# Phase 4 portability checkpoint

- Date: 2026-07-20
- Scope: Schema 15 API budget control plane and network-free OpenAI/Anthropic fixture contracts
- Result: passed on macOS, native Linux, and WSL2

## Safety conditions

The portability script removed `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, and `ANTHROPIC_AUTH_TOKEN` from the test process environment. The suite used fixture responses and fake executables only. It made no provider request and consumed no subscription quota or paid API credit. The two real-container tests remained explicitly ignored because this checkpoint did not opt in with digest-pinned image variables.

## macOS

- Kernel: Darwin 25.5.0 arm64
- Rust: 1.97.1
- Cargo: 1.97.1
- Strict Clippy: passed
- Tests: 115 passed, 0 failed, 2 explicitly ignored

The macOS suite was run directly because `scripts/test-phase4-portability` deliberately accepts only Linux and WSL2. This avoids presenting a Linux portability check as a macOS test while preserving the same format, lint, and fixture-only test gates.

## Linux VPS

- Kernel: Linux 6.8.0-107-generic x86_64 GNU/Linux
- Rust: 1.97.1
- Cargo: 1.97.1
- Script: `./scripts/test-phase4-portability`
- Strict Clippy: passed
- Tests: 115 passed, 0 failed, 2 explicitly ignored
- Terminal evidence: `Phase 4 quota-free portability checkpoint passed on linux`

## WSL2

- Platform detection: WSL2
- Script: `./scripts/test-phase4-portability`
- Tests shown in the supplied terminal tail: CLI JSON 7 passed; vertical slice 1 passed; doc tests passed
- Terminal evidence: `Phase 4 quota-free portability checkpoint passed on wsl2`

The script's terminal success is emitted only after format, strict Clippy, and the complete locked workspace test suite succeed with provider credential variables removed.

## Conclusion

P4-14 is established for the current intermediate scope: the normal macOS/Linux/WSL2 suites are fixture-only and quota-free. This is not Phase 4 exit evidence. Real provider transports, secret projection, routing integration, controlled extensions, and any explicitly opted-in paid smoke test remain incomplete.
