# Phase 4 live API checkpoint

- Date: 2026-07-21
- Platform: macOS host
- Scope: separately opted-in OpenAI and Anthropic response-only and isolated-patch smoke tests
- Result: operator reported all four exact ignored tests passed

## Executed matrix

| Provider | Test | Maximum requests | Retries | Output ceiling | Reported result |
| --- | --- | ---: | ---: | ---: | --- |
| OpenAI | Response-only transport and accounting | 1 | 0 | 32 tokens | Passed |
| OpenAI | Exactly scoped `submit_patch` plus detached verification | 1 | 0 | 512 tokens | Passed |
| Anthropic | Response-only transport and accounting | 1 | 0 | 32 tokens | Passed |
| Anthropic | Exactly scoped `submit_patch` plus detached verification | 1 | 0 | 512 tokens | Passed |

The operator ran the two repository scripts for each provider after explicitly supplying a protected credential reference, exact model ID, and the applicable literal one-request acknowledgement. The patch tests used only temporary repositories and state, allowed the provider only the built-in `submit_patch` tool, constrained changes to `result.txt`, passed independent detached-worktree verification, and asserted that the source checkout remained unchanged.

## Evidence boundary

The original scripts intentionally used temporary state and did not retain a receipt. This checkpoint therefore records the operator's explicit report, not independently inspectable machine output, raw provider response, usage record, or billing evidence. It must not be presented as Phase 4 exit evidence or as proof of a particular model's broader quality. No secret, response content, raw provider request ID, or private reasoning was supplied for this record.

Subsequent successful live-smoke runs write a private, redacted JSON receipt under `target/api-smoke-receipts/`. The receipt contains only provider, exact model ID, test kind, fixed request/retry/output limits, pass status, and timestamp. It is written only after the exact ignored Cargo test exits successfully. Failed or uncertain requests do not produce a passing receipt and must not be automatically retried.

## Conclusion

The live fixed-endpoint authentication, request construction, provider parsing, accounting settlement, typed patch result, isolated application, and independent verification paths have each been exercised once against both supported providers. Normal and portability suites remain credential-free and quota-free. Cross-platform fixture evidence and the controlled-extension acceptance work remain separate requirements.
