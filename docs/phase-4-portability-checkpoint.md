# Phase 4 portability checkpoint

- Date: 2026-07-20
- Evidence boundary: Git commit `5fdbb78` (`phase 4 pt12`)
- Scope: direct Rustls provider transport with no redirects, proxy inheritance, or implicit replay; separately ignored paid smoke-test gate; and the earlier Schema 19 request-plan, dispatch-attempt, API-budget, pricing, and protected-reference boundaries
- Result: passed on macOS, native Linux, and WSL2

## Safety conditions

The portability script removed `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`, and every real-API smoke selector and acknowledgement from the test process environment. The suite used fixture responses and fake executables only. It made no provider request and consumed no subscription quota or paid API credit. The two real-container tests and the one paid-API smoke test remained explicitly ignored.

## macOS

- Kernel: Darwin 25.5.0 arm64
- Rust: 1.97.1
- Cargo: 1.97.1
- Strict Clippy: passed
- Tests: 140 passed, 0 failed, 3 explicitly ignored

The macOS suite was run directly because `scripts/test-phase4-portability` deliberately accepts only Linux and WSL2. This avoids presenting a Linux portability check as a macOS test while preserving the same format, lint, and fixture-only test gates.

## Linux VPS

- Kernel: Linux 6.8.0-107-generic x86_64 GNU/Linux
- Rust: 1.97.1
- Cargo: 1.97.1
- Script: `./scripts/test-phase4-portability`
- Strict Clippy: passed
- Tests: 140 passed, 0 failed, 3 explicitly ignored
- Terminal evidence: `Phase 4 quota-free portability checkpoint passed on linux`

## WSL2

- Platform detection: WSL2
- Kernel: Linux 6.6.87.2-microsoft-standard-WSL2 x86_64 GNU/Linux
- Rust: 1.97.1
- Cargo: 1.97.1
- Script: `./scripts/test-phase4-portability`
- Strict Clippy: passed
- Tests: 140 passed, 0 failed, 3 explicitly ignored
- Terminal evidence: `Phase 4 quota-free portability checkpoint passed on wsl2`

The script's terminal success is emitted only after format, strict Clippy, and the complete locked workspace test suite succeed with provider credential variables removed.

## Conclusion

P4-14 is established for the direct-transport intermediate scope: the normal macOS/Linux/WSL2 suites are fixture-only and quota-free. The fixed-endpoint Rustls client, explicit network opt-in, disabled redirects/proxies/client retries, bounded response validation, sensitive authorization headers, and paid-smoke denial gate are portable across all three platforms. This is not Phase 4 exit evidence. General scheduler activation, a separately opted-in paid provider smoke test, and controlled extensions remain incomplete.
