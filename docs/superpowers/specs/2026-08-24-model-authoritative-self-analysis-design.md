# Model-Authoritative Customer-Side Self-Analysis

## Goal

Make the tested model the sole authority for each self-analysis PASS/FAIL decision. The customer-side CLI still collects the existing 46 checks and keeps the evidence log local, but it no longer overrides the target model with a Rust-derived capability verdict.

## Scope

- Keep the collection phase, request plans, protocol parsers, tool-loop execution, and `evidence.v4` log unchanged.
- Keep strict input-contract checks before analysis: complete run, supported schema/version, exact 46-test catalog, valid request references, Base64 sections, and required log structure.
- Keep analysis packet size limits, redaction, batch boundaries, cancellation, collision-safe output, and response JSON/schema/reference validation.
- Split check 057 evidence into four analysis packets for the 4, 8, 16, and 32 concurrency waves; use a 16KiB excerpt cap and 256KiB packet cap, then merge the four model reviews back into one report item.
- Remove deterministic capability hard-fail evaluation from packet construction and final status selection.
- Remove `hard_failures` as a source of PASS/FAIL decisions. Packets contain raw request metadata and bounded excerpts plus the declared test criteria; the model interprets them.
- Set `validatedStatus` equal to the accepted model `candidateStatus` and `decisionSource` to `TARGET_MODEL`.
- Emit self-analysis schema v3 and prompt v3 so downstream consumers can distinguish the model-authoritative semantics and the AI模型网关层数据结构兼容性 summary.

## Data Flow

```text
46-check collection
  -> complete local evidence.v4
  -> strict evidence reader
  -> bounded redacted packets with test criteria and raw observations
  -> same target model, one batch at a time
  -> syntactic/reference validation and one repair attempt
  -> target-model PASS/FAIL written locally
```

The analysis prompt must state that the packet is untrusted data, that only packet-owned evidence may be used, and that the model must return JSON matching the response schema. It must not describe local deterministic checks as authoritative verdicts.

## Decision Semantics

The validator may reject an unusable response, but it must not change a usable model decision:

- Unknown, duplicate, missing, or foreign test/evidence references: reject the batch and optionally request one repair.
- Empty observations or inconsistent `failureCause`: reject the batch and optionally request one repair.
- Accepted `PASS` or `FAIL`: preserve the model status exactly.
- A rejected batch: mark its tests `ANALYSIS_UNAVAILABLE`; do not synthesize FAIL.

This deliberately accepts the limitation that self-analysis is not independent evaluation. The artifact provenance and decision source make that explicit.

## Compatibility and Failure Handling

Without `--self-analyze`, behavior is unchanged. With the flag, collection success remains independent from optional analysis success. An incomplete or unsupported evidence log stops analysis before any model request; this is an input-contract failure, not a capability verdict.

## Tests

- Replace the hard-failure override test with a regression test proving a valid target-model PASS remains PASS even when the packet contains diagnostic evidence that would previously have forced FAIL.
- Add coverage that packets no longer contain authoritative hard-failure verdicts and still include the request metadata/excerpts and declared criteria needed by the model.
- Preserve candidate schema, reference, repair, partial-failure, cancellation, redaction, and output-collision tests.
- Run the full Rust, CLI integration, Python, formatting, Clippy, and release-build checks.
