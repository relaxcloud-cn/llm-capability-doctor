# Report Run Metadata Design

## Goal

Show the tested endpoint URL, requested model name, and a safely masked API-key
identifier near the top of every customer HTML report.

The report must remain auditable without storing or rendering a complete
credential.

## Source of Truth

The repository Skill under `skills/creating-model-doctor-reports` is the source
of truth. Implementation starts from the latest fetched `origin/main`; the
separate installed copy under `~/.codex/skills` is updated only after repository
tests and Skill validation pass.

## Credential Masking

The detector creates the display value before writing the run header:

- keys longer than eight characters retain the first four and final four
  characters, with the middle replaced by exactly eight asterisks;
- keys of eight characters or fewer become `[MASKED]`; and
- the complete key remains only in process memory and the existing secure
  redaction file used while the detector runs.

For example, a synthetic key `sk-test-12345678` is logged as
`sk-t********5678`. The fixed asterisk count avoids disclosing the original key
length.

The run-header field remains `api_key` for backward compatibility. Existing
logs containing `api_key: [REDACTED]` remain valid and render that literal value;
their credential fragments cannot be reconstructed.

## HTML Layout

The report renderer adds one unframed definition list before the capability
summary table. It contains exactly these customer-facing labels:

- `检测 URL`
- `模型名称`
- `API Key`

Values come only from validated assessment run metadata. Every value is HTML
escaped. Long URLs wrap within the report width, and the mobile layout stacks
labels above values without horizontal page overflow.

This run-information block is separate from capability status tables because
URL, model name, and credential identifier are test configuration, not model
capability conclusions.

## Safety and Redaction

The parser's existing second-pass redaction continues to remove credentials
from URL query parameters, JSON bodies, and HTTP headers. The masked run-header
identifier is allowed to survive parsing because it is already non-secret
display metadata.

The Skill workflow changes its final verification rule from forbidding every
credential-shaped value to forbidding unmasked credentials. Verification must
confirm that:

- the complete API key is absent from the source log, parsed JSON, assessment,
  and HTML;
- the HTML contains only the collector-provided masked identifier or the legacy
  `[REDACTED]` value; and
- query-string and header credentials remain fully redacted rather than partly
  masked.

No report-generation command accepts a raw API key override. This prevents a
reporting step from reintroducing a credential that was not present in the
audit log.

## Version and Compatibility

The detector version change is shared with the approved Anthropic output-budget
fix: `0.6.0` becomes `0.6.1`. No additional version increment is required.

Assessment schema structure does not change because the existing `run` object
already carries URL, model, and `api_key` metadata. Historical reports and logs
remain renderable.

## Testing

Test-driven coverage must establish the old behavior before implementation and
then verify:

- the detector logs a masked synthetic key and never logs the complete key;
- short synthetic keys become `[MASKED]`;
- the parser preserves a masked run-header identifier while continuing to redact
  full credentials from headers, URL queries, JSON, and echoed text;
- generated HTML displays all three labels and their escaped values;
- legacy `[REDACTED]` metadata still renders;
- long metadata values remain contained in desktop and mobile layouts;
- the Skill instructions require absence of unmasked credentials;
- the HTML remains self-contained with no external resources; and
- the complete repository test suite and Skill validator pass.

## Installation

After the feature branch is verified and integrated, replace the stale installed
Skill directory with a symlink to the repository's
`skills/creating-model-doctor-reports` directory. Preserve the existing
installation as a timestamped backup until the symlink and quick validation
succeed. This makes the fetched repository version authoritative while retaining
a rollback copy.

## Acceptance Criteria

The change is complete when a new `0.6.1` audit log contains only a masked
run-header API-key identifier, the generated HTML visibly presents URL, model,
and masked key above the capability tables, complete credentials are absent from
all artifacts, legacy logs still render, and the installed Skill resolves to the
verified repository source.
