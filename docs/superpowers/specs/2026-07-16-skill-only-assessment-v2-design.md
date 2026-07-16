# Skill-Only Assessment v2 Design

## Goal

Make the Skill-authored semantic review the only formal verdict in Model Doctor customer artifacts. Remove the collector script's mechanical status and every comparison against it from the canonical assessment JSON, customer HTML, summaries, documentation, and evaluation rules.

## Artifact Contract

The canonical schema version becomes `llm-capability-doctor.assessment.v2`.

Each assessment test keeps `reviewedStatus` as its only status field. The following v1 fields are removed:

- `originalStatus`
- `discrepancy`
- `originalTest`

Parsed-log summaries no longer expose `originalStatusCounts`. Customer HTML never renders a script verdict, comparison marker, discrepancy label, or discrepancy styling.

The source `.log` remains immutable. Temporary parsed evidence may faithfully contain status text found in that log because parsing is lossless evidence collection, but that status is not copied into either final artifact and is never treated as assessment ground truth.

## Data Flow

1. Parse and redact the source log without modifying it.
2. Let the Skill review observable inputs, outputs, protocol facts, and metrics.
3. Validate one `reviewedStatus` per discovered test.
4. Assemble `assessment.v2` without copying the parsed test result or original test object.
5. Derive category counts, blockers, conditions, and the overall verdict exclusively from `reviewedStatus`.
6. Render the customer HTML exclusively from the v2 reviewed status and conclusion.

## Compatibility

This is an intentional breaking schema change. Existing `assessment.v1` files are historical artifacts and are not silently upgraded or accepted as new canonical output. Generate v2 JSON and HTML again from the original audit log and semantic reviews.

The collector script and source log format do not change. Request inputs, outputs, metrics, evidence references, limitations, and retest instructions remain available in v2.

## Documentation Rules

The Skill and evaluation rules must state that parsed script results are untrusted evidence, not formal verdicts. They must not instruct agents to preserve, compare, display, or explain script-versus-Skill discrepancies.

Customer-facing documentation must describe the report as a Skill semantic assessment. It must not advertise original script verdicts, discrepancy filtering, or change markers.

## Validation

Automated tests must prove that:

- v2 assessment tests contain `reviewedStatus` and do not contain `originalStatus`, `discrepancy`, or `originalTest`;
- summary output does not contain `originalStatusCounts`;
- HTML for a parsed PASS reviewed as FAIL shows only the reviewed FAIL result and contains no script verdict or change marker;
- category counts and overall gates still derive from reviewed statuses;
- the schema, Skill structure, renderer safety, offline CSP, escaping, and output collision behavior remain valid;
- the complete regression suite passes.

Generate a new timestamped customer report for browser verification. Never overwrite the existing source log, assessment, or HTML reports.
