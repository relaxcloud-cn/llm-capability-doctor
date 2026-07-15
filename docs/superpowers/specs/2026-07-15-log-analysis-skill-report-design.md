# LLM Capability Doctor Log Analysis Skill and Customer Report Design

## 1. Purpose

`llm-capability-doctor` remains a lightweight black-box probe. A user runs the
shell script with model connection information, obtains one complete audit log,
and then gives that log to a Codex skill for analysis.

The skill turns the audit log into two outputs:

1. A canonical machine-readable assessment.
2. A self-contained customer-facing HTML readiness report.

The design deliberately separates evidence collection from semantic judgment.
The shell script records what was sent and received. The skill decides what the
evidence means.

## 2. Goals

- Accept one complete Model Doctor `.log` file as the only required input.
- Preserve the original log unchanged.
- Evaluate every discovered test using explicit, test-specific rubrics.
- Distinguish transport observations from model capability judgments.
- Preserve both the script's original result, when present, and the skill's
  reviewed result.
- Produce customer-facing conclusions that cite concrete request and response
  evidence.
- Show test logic, inputs, outputs, metrics, limitations, and retest advice.
- Generate deterministic report structure from a canonical assessment object.
- Treat missing, truncated, contradictory, or ambiguous evidence as
  `UNDETERMINED` rather than guessing.
- Keep credentials and sensitive headers out of generated artifacts.

## 3. Non-Goals

- The skill does not call the tested model again.
- The skill does not modify or append to the original log.
- The shell script does not need to implement general semantic output parsing.
- The report does not prove the upstream model's commercial identity from a
  model name alone.
- The endpoint-only workflow does not certify the complete Codex app-server,
  MCP, plugin, skill, approval, or ClawOps runtime chain.
- The report does not expose private chain-of-thought. It explains conclusions
  through observable evidence and concise reasoning.

## 4. Selected Architecture

```text
model-capability-doctor.sh
        |
        | complete audit log
        v
llm-readiness-report skill
        |
        | parse blocks, review evidence, apply rubrics
        v
assessment.json
        |
        | deterministic renderer
        v
customer-readiness-report.html
```

The skill orchestrates analysis and rendering, but the assessment is created
before the HTML. The HTML is a view of the assessment, not an independently
written narrative.

## 5. Workflow

1. The user runs `model-capability-doctor.sh`.
2. The script writes a complete audit log containing the run header, request
   blocks, test blocks, raw request and response content, metrics, and run
   summary.
3. The user invokes the report skill and supplies the log path.
4. The skill validates that the file is readable and resembles a Model Doctor
   audit log.
5. The skill inventories run metadata, requests, tests, and summary counts.
6. The skill applies the rubric for each discovered test.
7. The skill performs a consistency review across related requests and tests.
8. The skill writes `<log-stem>-assessment.json` beside the log.
9. The skill renders `<log-stem>-customer-readiness-report.html` beside the
   log.
10. The skill reports both output paths and a concise overall verdict.

The generated files never replace the input log or an existing report. If the
default output name already exists, the skill adds a timestamp suffix.

## 6. Input Log Interpretation

The log is the source of evidence. The skill recognizes the current block
markers:

- `MODEL DOCTOR RUN`
- `REQUEST <id> BEGIN/END`
- `TEST-<id> BEGIN/END`
- `RUN SUMMARY`

For each request, the skill should recover when present:

- Request ID and related test ID.
- Timestamp, protocol, authentication mode, and stream flag.
- Redacted URL and requested model.
- Complete request body.
- HTTP status, curl exit code, total time, first-byte time, and download size.
- Response headers, stderr, and complete response body.

For each test, the skill should recover when present:

- Test ID, name, category, original status, expectation, detected value, and
  script conclusion.
- Raw response evidence included in the test block.
- All request blocks associated with that test, including follow-up, repeated,
  and parallel requests.

Unknown blocks are preserved as unclassified evidence and do not cause the
whole analysis to fail.

## 7. Assessment Contract

The assessment uses schema ID `llm-capability-doctor.assessment.v1` and contains:

### 7.1 Run Metadata

- Source log file name, file size, and SHA-256. Generated customer artifacts do
  not expose the source file's absolute local path.
- Run ID, script version, start and completion times.
- Redacted endpoint, requested model, observed returned model when available.
- Detected protocol, timeout, request count, and run duration.
- Total input, output, reasoning, and combined tokens when observable.
- Parser warnings and evidence completeness.

### 7.2 Per-Test Assessment

Each test includes:

- `testId`, `category`, and `name`.
- `gateLevel`: `critical`, `important`, or `observation`.
- `originalStatus` and `reviewedStatus`.
- `reviewedStatus`: `PASS`, `FAIL`, `UNSUPPORTED`, `UNDETERMINED`, `SKIPPED`,
  or `ERROR`.
- `confidence`: `high`, `medium`, or `low`.
- Customer-facing conclusion.
- Detection purpose, method, pass criteria, failure criteria, and capability
  boundary.
- Observable metrics.
- Evidence references pointing to request and response blocks.
- Short evidence excerpts plus full redacted request and response content.
- Limitations, discrepancies, and retest instructions.

### 7.3 Category Assessment

Each category includes status counts, critical failures, unknowns, key metrics,
a concise conclusion, and readiness impact.

### 7.4 Overall Assessment

The overall verdict is one of:

- `READY`: every critical gate passes and no critical result is unknown.
- `CONDITIONAL`: no critical gate fails, but at least one critical or important
  capability remains unknown, unsupported, unstable, or requires retesting.
- `BLOCKED`: at least one critical gate fails or the log is too incomplete to
  establish endpoint readiness.

A numerical average never overrides a critical gate.

## 8. Evaluation Rules

### 8.1 Evidence First

Every reviewed conclusion must cite observable evidence. HTTP success alone
does not prove semantic success. Text that merely mentions a tool name does not
prove a formal tool call.

### 8.2 Raw and Reviewed Results

When the skill disagrees with the script, the report must show:

```text
Original result -> Reviewed result -> Reason -> Evidence
```

The original result is never silently replaced.

### 8.3 Confidence

- `high`: direct, complete, protocol-level or exact semantic evidence.
- `medium`: sufficient semantic evidence with limited ambiguity.
- `low`: incomplete evidence that supports a tentative conclusion.

Low-confidence positive evidence cannot promote a critical test to `PASS`; it
becomes `UNDETERMINED` with retest guidance.

### 8.4 Stability Language

A single successful sample proves only that the observed request succeeded.
Words such as "stable", "reliable", and "production-ready" require repeated
samples that meet an explicit threshold.

### 8.5 Protocol and Semantic Variation

Natural-language output may vary across models and is judged semantically.
Protocol envelopes, tool-call structures, call IDs, SSE completion events, and
usage fields are judged against the protocol claimed by the endpoint.

## 9. Customer Report Information Architecture

The report is a self-contained, offline HTML document with no external CDN,
font, script, image, or network dependency.

### 9.1 First Viewport

The first viewport answers four questions immediately:

1. Is the endpoint ready for ClawOps-style agent use?
2. What blocks or conditions that decision?
3. Which capabilities were actually verified?
4. What must be retested?

It contains:

- Report title, model, protocol, run time, and script version.
- Overall `READY`, `CONDITIONAL`, or `BLOCKED` verdict.
- A plain-language readiness statement.
- Critical blockers, unknowns, and retest actions.
- Compact counts for passed, failed, unknown, unsupported, and error states.

### 9.2 Capability Summary

Show one compact row per category with:

- Category status.
- Passed and total reviewed tests.
- Critical gate state.
- Key metrics.
- Customer-facing conclusion.

Categories are not assigned equal weight. Critical gate impact remains visible.

### 9.3 Methodology

Explain:

- The black-box connection-only boundary.
- What the script collected.
- How the skill reviewed it.
- Status and confidence definitions.
- Character-versus-token and first-byte-versus-first-token limitations.
- Why endpoint readiness is narrower than full ClawOps runtime certification.

### 9.4 Test Detail

Tests are grouped by category rather than repeating the category in every row.
The collapsed row shows:

- Test ID and name.
- Raw observation.
- Reviewed status and gate level.
- Short conclusion.
- Discrepancy marker when original and reviewed states differ.

Expanding a test shows:

1. Detection purpose.
2. Test method and input design.
3. Pass and failure criteria.
4. Capability boundary.
5. Raw metrics.
6. Complete redacted request input.
7. Complete redacted model or protocol output.
8. Evidence excerpts used by the skill.
9. Reviewed conclusion, confidence, limitations, and retest instructions.

Multi-turn and tool tests display a request timeline instead of one combined
blob. Repeated and parallel tests display a sample table with per-request HTTP
status, latency, and semantic success when observable.

### 9.5 Filtering and Navigation

- Sticky section navigation.
- Filters for status, category, gate level, and reviewed discrepancies.
- Default emphasis on failures, unknowns, errors, and changed judgments.
- Expand and collapse controls for all test evidence.
- Print styles that preserve conclusions while keeping raw evidence compact.

## 10. Input and Output Evidence Presentation

Inputs and outputs are included because they make the report auditable.

- Normal-size content is embedded in full and collapsed by default.
- Large context and long-output cases show size, hash, key positions, and
  evidence excerpts before the full content.
- Full content remains available through an explicit expand control.
- Raw JSON and SSE are formatted as text without changing their content.
- No content from the log is executed as HTML, Markdown, or JavaScript.

The report explains detection logic in customer language. It does not paste
shell implementation details or private evaluator reasoning.

## 11. Security and Privacy

The skill performs a second redaction pass even if the script already redacted
the log.

It redacts:

- Authorization, proxy authorization, API key, and cookie headers.
- Common URL credential query parameters.
- Secret-like values discovered in credentials-bearing headers, URL query
  parameters, or JSON fields, including echoed copies of those values.
- Obvious secret fields such as `api_key`, `access_token`, `client_secret`, and
  `password`.

All embedded request and response text is HTML-escaped. Content Security Policy
for the report disallows network access, external scripts, forms, frames, and
object embedding.

The report states that it may still contain model prompts and business data and
must be reviewed before external distribution.

## 12. Error Handling

- Unreadable or empty log: stop without creating a misleading report.
- Invalid run header: explain that the file is not a recognized Model Doctor
  log.
- Missing request block: mark affected tests `UNDETERMINED`.
- Truncated request or response: preserve available evidence, flag truncation,
  and avoid positive capability claims.
- Missing run summary: continue with reconstructed counts and add a warning.
- Unknown test ID: include it under an `Unclassified` category without guessing
  its rubric.
- Rendering failure: keep the valid assessment JSON and report the failure.
- Existing output path: create a timestamped output instead of overwriting.

## 13. Validation Strategy

The implementation must be verified with fixture logs covering:

- A complete successful run.
- Mixed `PASS`, `FAIL`, `UNSUPPORTED`, `UNDETERMINED`, and `ERROR` results.
- A script result that the skill legitimately overturns.
- Missing and truncated blocks.
- Multi-turn tool-call evidence.
- Parallel and repeated performance evidence.
- Very large request and response bodies.
- Secrets echoed in headers, URLs, requests, and responses.
- HTML and script injection text in model output.
- An unknown future test ID.

Acceptance requires:

- Original logs remain byte-for-byte unchanged.
- Assessment JSON conforms to the v1 schema.
- Every test verdict has evidence or an explicit unknown reason.
- HTML contains no unredacted fixture secrets.
- HTML works offline and makes no network requests.
- Raw log text is rendered inert.
- Overall verdict follows the critical-gate rules.
- Report structure and status counts match the assessment JSON.

## 14. Implementation Scope

The first implementation delivers:

- A reusable Codex skill packaged according to the current Codex skill
  contract.
- Test-specific rubrics for the current 62-item catalog.
- A parser for the current Model Doctor audit-log block format.
- The `llm-capability-doctor.assessment.v1` JSON contract.
- A deterministic, self-contained HTML renderer.
- Fixture logs and automated contract tests.
- User documentation showing how to run the script and invoke the skill with a
  log file.

Changes to the probe catalog may be made only when they improve the quality of
the generated evidence. Complex semantic judgment remains owned by the skill.
