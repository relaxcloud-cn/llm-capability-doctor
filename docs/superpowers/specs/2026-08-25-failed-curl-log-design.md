# Failed cURL Log Design

## Goal

When `--self-analyze` completes, create a second log containing the replay cURL
commands associated with checks whose validated status is `FAIL`. The existing
audit log remains the complete evidence record.

## Output

The file is placed beside the audit log. Its name inserts `-failed-curls` before
the original extension:

- `model-doctor.log` becomes `model-doctor-failed-curls.log`.
- `audit` becomes `audit-failed-curls.log`.

The CLI reports this path and the number of failed checks after self-analysis.
The file is created even when no checks fail and declares `failed_test_count: 0`.

## Contents

The failed-cURL log has a run header followed by one section per failed check.
Each section contains the check ID, name, category, analysis failure cause,
referenced request IDs, and the cURL command section for each referenced
request. A request shared by more than one failed check appears in each check's
section so the reproduction context remains complete.

Only results with validated status `FAIL` are included. `PASS` and
`ANALYSIS_UNAVAILABLE` results are excluded.

## Data Flow

The self-analysis artifact already identifies failed checks and their evidence
request references. The writer reads the completed audit log and extracts the
existing `CURL COMMAND` blocks for those request IDs. It does not synthesize a
new command, so the generated log preserves the actual protocol, streaming,
timeout, TLS, and already-redacted authentication details from the audit log.

## Error Handling and Security

The failed-cURL log uses the project's private-file helper. A missing referenced
request or missing cURL block is reported as an analysis-output error rather
than producing an incomplete log. Extracted content comes from the already
redacted audit log; no API key is read from CLI configuration or written to the
new file.

## Tests

Tests cover output naming, `FAIL` filtering, extraction of the matching cURL
block, preservation of per-test grouping for shared requests, the empty-failure
log, and rejection of malformed source evidence.
