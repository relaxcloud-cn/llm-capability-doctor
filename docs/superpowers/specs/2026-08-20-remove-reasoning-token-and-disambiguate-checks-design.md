# Remove Reasoning Token And Disambiguate Checks

## Goal

Ship one new, clean Model Doctor contract that removes check 034 and makes checks 020 and 022 mechanically unambiguous. Historical evidence and assessment contracts are no longer accepted or documented.

## Contract

The collector becomes `v0.12.0` and writes `llm-capability-doctor.evidence.v4`. The report pipeline emits `llm-capability-doctor.assessment.v9`. The new contract contains exactly 46 checks: 32 core checks and 14 enhanced checks.

Only the exact `evidence.v4` and `v0.12.0` pair is accepted. Existing `evidence.v1`, `evidence.v2`, and `evidence.v3` logs are rejected as unsupported inputs. No compatibility migration or reinterpretation is provided.

Check 034 is removed from the Rust catalog, request planner, report display catalog, evaluation rules, verdict partitions, schemas, reference shell catalog, documentation, and tests. Reasoning text separation and streaming checks 033, 035, and 036 remain unchanged.

## Unambiguous Checks

Check 020 will request an exact three-line payload and explicitly state that the two ASCII vertical-bar characters (`|`) are literal output characters. The evaluator will continue to compare the exact three lines after normalizing line endings and allowing one terminal newline.

Check 022 will provide direct source fields including `labels=URGENT,DATABASE` and `excluded_label=NETWORK`. It will instruct the model to copy only the comma-separated `labels` field, preserve its order, and never infer labels from other fields. The evaluator will require the same four-field JSON and exactly the two requested labels.

## Report Behavior

Reports will show 46 total checks. Check 034 will never appear in the capability table, issue summary, counts, or general verdict. A complete run passes generally when all 46 checks pass, is conditional when all 32 core checks pass but at least one of the 14 enhanced checks fails, and fails when a core check fails.

OpenCodex data-format compatibility keeps the same eight hard gates and is unaffected by removing 034.

## Verification

Tests will first fail against the old 47-check contract, old ambiguous prompts, and visible check 034. The implementation will then update the collector, parser, assessment assembler/validator, renderer, skill instructions, schemas, fixtures, and README. Verification requires the complete Rust and Python suites, a release build, a generated new-contract fixture report, and searches proving that no active catalog, rule, prompt, display mapping, or report output contains check 034.

## Non-Goals

This change does not alter the behavior or scoring of checks other than 020, 022, and removal of 034. It does not regenerate historical reports because their source logs use unsupported contracts and cannot contain responses to the new prompts.
