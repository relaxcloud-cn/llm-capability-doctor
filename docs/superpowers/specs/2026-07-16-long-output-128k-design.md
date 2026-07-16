# Long Output 128K Ladder Design

## Goal

Expand the current Model Doctor long-output coverage from two levels to five levels: 8K, 16K, 32K, 64K, and 128K visible Result JSON. Raise the default per-request timeout to 120 seconds so slow long-output requests have a meaningful opportunity to complete.

## Catalog And Version

Publish the change as script version `0.3.0` with a 65-item core catalog.

Use these consecutive long-output test IDs:

| ID | Test |
| --- | --- |
| `014` | 8K complete Result JSON |
| `015` | 16K complete Result JSON |
| `016` | 32K complete Result JSON |
| `017` | 64K complete Result JSON |
| `018` | 128K complete Result JSON |

Shift every existing test from `016–062` forward by three positions to `019–065`. Preserve its category, name, prompt semantics, handler behavior, evidence, and pass criteria. The resulting catalog IDs must be continuous and unique from `001` through `065`.

Range migration:

| Existing IDs | New IDs | Category |
| --- | --- | --- |
| `016–022` | `019–025` | Instruction and text |
| `023–031` | `026–034` | Context |
| `032–039` | `035–042` | Thinking and reasoning |
| `040–050` | `043–053` | Tool calls |
| `051–058` | `054–061` | Performance and stability |
| `059–062` | `062–065` | Guardrails and vocabulary |

## Long-Output Requests

Run the five levels as independent tests in ascending order. Continue to later levels even when an earlier level fails, errors, times out, or is unsupported; the report must expose the observed result at every requested boundary.

For each level:

1. Ask for one JSON object with the existing Result fields, three investigation stages, six evidence items, evidence references, deterministic padding, and a level-specific final completion marker.
2. Set the protocol-specific output limit to `target_tokens + 1024`.
3. Validate protocol success, truncation signals, the level-specific completion marker, required structure, visible byte plausibility, and reported visible output tokens.
4. Record the request, response, metrics, evidence, and conclusion under that level's test ID.

Generate the request label and completion marker from the target size instead of branching only between 8K and 16K. Supported target values are exactly `8192`, `16384`, `32768`, `65536`, and `131072`.

## Timeout Behavior

Change the CLI default `TIMEOUT_SECONDS` from 90 to 120 and update `--help` and README text. The existing `--timeout SECONDS` option remains the single override for all requests, including long-output tests. A user may still choose a larger value such as 300 seconds for especially slow endpoints.

The final handoff command will pass `--timeout 120` explicitly so the audit log records an unambiguous execution configuration.

## Compatibility

This intentionally changes test IDs `016–062`. New logs use script version `0.3.0` and the new 65-item catalog. Old logs and reports remain immutable historical artifacts and continue to parse using their embedded catalog metadata.

The report Skill requires no special-case rendering changes because it discovers test IDs and categories from the log. A newly executed log must be analyzed again to produce a new assessment and customer report.

## Validation

Add automated script-contract tests that execute `--list-tests` and `--help` without contacting a model endpoint. They must prove:

- the version is `0.3.0`;
- help advertises a 120-second default and a 65-item catalog;
- catalog IDs are exactly `001–065`, with no duplicates or gaps;
- `014–018` map to 8K, 16K, 32K, 64K, and 128K long-output tests;
- representative shifted tests retain their names at the new IDs;
- the source maps all five long-output IDs to the long-output handler and defines all five target sizes;
- existing Python report and Skill tests still pass;
- `bash -n`, Skill validation, and whitespace checks pass.

Do not execute paid or external model requests during automated verification. The user will run the final one-line command against the supplied endpoint and provide the new log for semantic assessment.
