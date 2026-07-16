# Context-Only Capacity Ladder Design

## Goal

Replace the long-output capability ladder with one consistent input-context
capacity ladder. Model Doctor will test whether an endpoint can accept large
inputs and retrieve a short hidden marker. It will no longer attempt to prove
that a model can generate 8K, 16K, 32K, 64K, or 128K output tokens.

The change publishes script version `0.4.0` with a 62-item core catalog.

## Catalog

Use five consecutive context-capacity tests:

| ID | Category | Test |
| --- | --- | --- |
| `014` | Context | 8K-level input context (character approximation) |
| `015` | Context | 16K-level input context (character approximation) |
| `016` | Context | 32K-level input context (character approximation) |
| `017` | Context | 64K-level input context (character approximation) |
| `018` | Context | 128K-level input context (character approximation) |

Remove the duplicate context-capacity entries currently at `026-028`. Keep the
instruction tests at `019-025`. Shift the remaining tests that currently start
at `029` backward by three positions:

| Current IDs | New IDs | Category |
| --- | --- | --- |
| `029-034` | `026-031` | Context position, interference, and multi-turn |
| `035-042` | `032-039` | Thinking and reasoning |
| `043-053` | `040-050` | Tool calls |
| `054-061` | `051-058` | Performance and stability |
| `062-065` | `059-062` | Guardrails and vocabulary |

The resulting IDs must be continuous and unique from `001` through `062`.
There is no `long output` category or long-output handler in the new catalog.

## Context-Capacity Requests

All five levels use the same request and assessment logic. The target level is
converted to an approximate character payload using the existing four
characters-per-token convention:

| Level | Generated filler characters |
| --- | ---: |
| 8K | 32,000 |
| 16K | 64,000 |
| 32K | 128,000 |
| 64K | 256,000 |
| 128K | 512,000 |

For each test:

1. Generate deterministic filler locally.
2. Build one protocol-compatible, non-streaming request containing the filler
   and a level-specific hidden marker at the end.
3. Ask the model to reply only with that marker.
4. Record the exact request, response, curl metrics, HTTP status, and protocol
   usage fields.
5. Pass only when the request succeeds and the model-visible answer contains
   the exact marker.

The conclusion reports the actual character count and observed input-token
count separately when the protocol returns usage. Character count is never
presented as an exact token count. A successful test establishes only an
observed lower bound for this payload and interaction; it does not establish
the endpoint's maximum context window.

## Failure Semantics

- Transport or timeout failures are `ERROR`.
- Explicit HTTP rejection of the context payload, including `400`, `413`, or
  `422`, is `FAIL` unless the protocol provides a more specific unsupported
  signal already handled by the script.
- A successful HTTP response with a missing or incorrect marker is `FAIL`.
- An unknown protocol that prevents extraction of the model-visible answer is
  `UNDETERMINED`.
- Missing input-token usage does not fail an otherwise complete marker-recall
  result; the report states that only the character payload was observed.

The script continues to later capacity levels after any earlier failure,
timeout, or rejection so the report preserves every observed boundary.

## Removed Long-Output Behavior

Delete the long-output request builder, output-token counting, truncation
checks, completion-marker validation, long-output globals, and long-output
dispatch. No replacement streaming-output test is introduced. This is an
intentional product decision: the catalog measures input context only.

Remove README and help text that describes 8K-128K generated Result JSON or
recommends long-output timeout settings. Keep the general `--timeout` option
for all remaining requests.

## Gate Levels And Reporting

The fixed `0.4.0` priority map is:

- `critical`: `001-006`, `040-050`;
- `important`: `014-018`, `032-036`, `057`;
- `observation`: all remaining IDs.

Update both the shell catalog contract tests and the report skill's evaluation
rules and validator mapping. The HTML still combines `critical` and
`important` as important checks.

The catalog contains 28 displayed important checks: 17 `critical` IDs plus 11
`important` IDs (five capacity tests, five Thinking protocol tests, and one
concurrency test). The remaining 34 checks are observations. Report copy and
tests must derive these counts from the final mapping rather than preserve the
old hard-coded 26-item statement.

## Compatibility

Existing `0.2.0` and `0.3.0` logs and generated reports remain immutable
historical artifacts. The parser continues to discover tests from each log.
Evaluation rules retain explicit historical mappings so old logs are assessed
against the catalog that produced them.

New runs use `0.4.0`, the 62-item catalog, and the context-only gate map. A new
run must be analyzed again to produce a matching assessment and customer HTML
report; existing reports are not rewritten.

## Validation

Use test-driven development and avoid paid or external model requests during
automated verification. Tests must prove:

- `--help` reports version `0.4.0` and a 62-item catalog;
- `--list-tests` returns exactly `001-062` without gaps or duplicates;
- `014-018` are the five context-capacity levels;
- the source maps those levels to the context handler and defines the exact
  character targets;
- no long-output builder, handler, dispatch, completion markers, or stale
  long-output globals remain;
- representative shifted test IDs and internal markers are correct;
- context conclusions keep character counts separate from observed input
  tokens;
- the report validator enforces the `0.4.0` gate map while retaining historical
  behavior for older logs;
- README and help text describe the context-only ladder accurately;
- shell syntax, Python tests, skill validation, and whitespace checks pass.
