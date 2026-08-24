# Model-Authoritative Self-Analysis Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make the tested model the sole source of customer-side self-analysis PASS/FAIL decisions while preserving evidence and response-contract safety checks.

**Architecture:** Keep collection and `evidence.v4` unchanged. Build bounded redacted packets from raw request evidence and the declared per-test criteria, without local capability verdicts. Validate only the model response schema and references, then preserve the accepted model status as the final status.

**Tech Stack:** Rust, Tokio, Serde/Serde JSON, existing CLI and evidence reader, Cargo test/Clippy.

---

### Task 1: Lock target-model decision semantics with tests

**Files:**
- Modify: `src/analysis/validator.rs`
- Modify: `src/analysis/packet.rs`

- [x] Add a validator regression test where a valid model `PASS` remains `PASS` and `decisionSource` remains `TARGET_MODEL`; remove the old expectation that a packet diagnostic forces `FAIL`.
- [x] Add a packet regression test proving a packet carries `passCriteria`, request metadata, and bounded excerpts without an authoritative hard-failure verdict field.
- [x] Run `cargo test analysis::validator::tests analysis::packet::tests --lib` and confirm the new tests fail against the current implementation.

### Task 2: Remove analysis-layer capability hard gates

**Files:**
- Modify: `src/analysis/rules.rs`
- Modify: `src/analysis/packet.rs`
- Delete: `src/analysis/projection.rs`
- Modify: `src/analysis/mod.rs`

- [x] Reduce `AnalysisRule` to the test ID and declared `passCriteria`; keep the 46 textual criteria and rule/catalog completeness test.
- [x] Build packets directly from parsed requests, preserving request metadata, timing fields, and bounded request/response/stderr excerpts; remove `deterministic_failures`, marker gates, structured-value gates, metric gates, interface gates, and tool-history verdict gates from this analysis path.
- [x] Remove the unused analysis-only projection module and its tests; collection protocol parsing and tool validation remain owned by `src/protocol` and `src/runner.rs`.
- [x] Update packet tests to cover packet ownership, limits, UTF-8-safe compaction, and batching without asserting local PASS/FAIL judgments.
- [x] Run the focused analysis tests and then `cargo test --all-targets`.

### Task 3: Preserve accepted model status and update prompt/output semantics

**Files:**
- Modify: `src/analysis/validator.rs`
- Modify: `src/analysis/prompt.rs`
- Modify: `src/analysis/orchestrator.rs`

- [x] Make `validate_candidates` retain each accepted `candidateStatus` as `validatedStatus` and always use `DecisionSource::TargetModel`.
- [x] Keep rejection rules for missing/duplicate/foreign IDs, empty observations, and inconsistent failure causes; preserve one repair attempt and `ANALYSIS_UNAVAILABLE` for unresolved batches.
- [x] Update prompt wording so the model is explicitly responsible for deciding PASS/FAIL from the packet evidence, without presenting local diagnostic verdicts as authoritative.
- [x] Update orchestrator tests and artifact assertions for target-model decisions and unchanged unavailable-batch behavior.

### Task 4: Verify compatibility and deliver

**Files:**
- Modify: `README.md` if the customer-facing self-analysis wording still describes local rule overrides.

- [x] Run `cargo fmt --check`, `git diff --check`, `cargo test --all-targets`, `cargo clippy --all-targets -- -D warnings`, and `cargo build --release`.
- [x] Run `python3 -m unittest discover -s skills/creating-model-doctor-reports/tests -p 'test_*.py'`.
- [ ] Inspect the final diff and commit the implementation with a focused message.
