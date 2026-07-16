# Anthropic Output Budget Design

## Goal

Prevent false capability failures when an Anthropic-compatible endpoint counts
Thinking content and the visible final answer against the same output-token
budget.

The detector must give short deterministic answers enough room to complete even
when the endpoint enables Thinking by default. The change raises the request
ceiling; it does not disable or hide the model's default reasoning behavior.

## Root Cause

`protocol_body()` currently emits `max_tokens: 64` for Anthropic Messages
requests. `core_multi_turn_body()` repeats the same limit, while
`core_tool_body()` uses a separate limit of `256`.

The observed `deepseek-v4-pro[1m]` audit log shows the consequence directly:

- affected requests return HTTP `200` with curl exit code `0`;
- `usage.output_tokens` reaches exactly `64`;
- `stop_reason` is `max_tokens`; and
- the visible answer is absent or contains only a prefix of the requested exact
  marker.

The same endpoint completes the tool-call tests that use the larger `256`
ceiling. This isolates the failure to the detector's output budget rather than
transport, authentication, context acceptance, or prompt construction.

## Chosen Approach

Define one shell constant:

```bash
ANTHROPIC_MAX_TOKENS="2048"
```

Every Anthropic Messages request builder uses this value. This includes:

- ordinary synchronous and streaming requests built by `protocol_body()`;
- multi-turn context requests built by `core_multi_turn_body()`; and
- tool-call requests built by `core_tool_body()`.

Explicit Thinking tests continue to request `budget_tokens: 1024` and inherit
the `2048` overall output ceiling from `protocol_body()`. This leaves room for a
visible answer after the requested Thinking budget.

The script contract version advances from `0.6.0` to `0.6.1` because generated
request bodies and resulting assessment evidence change.

## Alternatives Rejected

Adding a `--max-output-tokens` option would support site-specific tuning, but it
would enlarge the public CLI and validation surface without being necessary to
correct the known false negatives.

Retrying requests after `stop_reason=max_tokens` would reduce output allowance
on successful calls, but retries would invalidate cold latency, repeated
success, concurrency, and recovery measurements. The audit would also contain
two different attempts for one nominal sample.

Disabling Thinking on non-Thinking tests would change the endpoint's default
behavior and might not be supported consistently by Anthropic-compatible
providers. The detector should observe the configured model rather than alter
its reasoning mode to fit a small collector budget.

## Scope

This change:

- centralizes the Anthropic output limit in one named constant;
- replaces the current Anthropic `64` and `256` request limits with `2048`;
- updates the version shown in help and audit-log metadata to `0.6.1`;
- preserves all 62 test IDs, prompts, timeouts, context payloads, gate levels,
  response graders, and logging behavior; and
- leaves OpenAI Chat Completions, OpenAI Responses, Gemini, and Ollama request
  limits unchanged.

This change does not add retries, a new command-line option, adaptive budgeting,
or a provider-specific Thinking-disable flag.

## Request and Evidence Behavior

The output ceiling is a maximum, not a request to generate 2048 tokens. Exact
marker prompts should still stop as soon as the endpoint completes the answer.
Providers that generate long Thinking content may consume more tokens than
before, which is the accepted trade-off for avoiding collector-induced
truncation.

The complete redacted request body remains in every audit block. Reviewers can
therefore verify `max_tokens: 2048` directly and distinguish an endpoint-side
completion limit from the previous detector-side ceiling.

If an endpoint still returns `stop_reason=max_tokens` at `2048`, the existing
raw response and usage fields remain available for semantic review. This change
does not claim that 2048 is an unlimited output budget.

## Error Handling

The existing transport, HTTP, protocol, and semantic status rules remain
unchanged. Increasing the ceiling does not convert HTTP success into semantic
success; each test must still return the required visible output or formal tool
call.

The constant is an internal positive integer and is serialized as a JSON number.
No user input or new validation path is introduced.

## Testing

Regression coverage will use the existing fake curl harness to force Anthropic
Messages protocol detection and inspect real audit request blocks. Tests will
verify that:

- ordinary Anthropic requests contain `"max_tokens":2048`;
- multi-turn Anthropic requests contain `"max_tokens":2048`;
- Anthropic tool-call requests contain `"max_tokens":2048`;
- explicit Thinking requests retain `budget_tokens: 1024` while using the
  `2048` overall output ceiling;
- no affected Anthropic builder retains a hard-coded `64` or `256` ceiling;
- help output and source metadata report version `0.6.1`;
- shell syntax validation passes; and
- the complete Python test suite passes.

The regression test must fail against version `0.6.0` because its logged
Anthropic request bodies contain the old ceilings. Only after observing that
failure will the production script be modified.

## Acceptance Criteria

The fix is complete when one Anthropic-compatible run records `max_tokens: 2048`
for ordinary, multi-turn, Thinking, streaming, performance, guardrail, and tool
requests; explicit Thinking retains its `1024` budget; version `0.6.1` is visible
in help and logs; and all automated checks pass.

The existing `deepseek-v4-pro[1m]` log remains unchanged. A new run is required
to reassess the cases previously truncated at 64 output tokens.
