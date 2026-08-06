# Single Full Collection Design

## Goal

Make every normal collector run execute the complete 46-check catalog. Remove the
onsite, full, and custom-selection concepts from the public CLI and from new audit
logs.

## Command Contract

The Rust CLI no longer accepts `--full` or `--only`. A command with neither flag
runs all 46 checks. Passing either removed flag produces Clap's unknown-argument
error so scripts do not silently run a different workload.

The legacy shell collector also removes `--only`; it continues to execute its
complete catalog. `--list-tests` remains available in both collectors because it
only prints the catalog and does not select a run profile.

## Collector Architecture

`Config` contains endpoint, authentication, output, timeout, and TLS settings only.
It no longer contains a selected ID list or collection profile. The Rust runner
always obtains the full catalog with `catalog::all()`.

The onsite catalog constant and selector are deleted. `PlanContext` no longer has
an `onsite` flag. Check 057 always plans four concurrent batches with sizes 4, 8,
16, and 32, for 60 requests in that check.

## Audit Format Migration

Removing `collection_profile` changes the audit contract, so the Rust collector
version moves from `0.9.0` to `0.10.0` and new logs declare
`llm-capability-doctor.evidence.v2`. New v2 run headers omit
`collection_profile` entirely and always contain all 46 manifests.

The reporting parser keeps explicit compatibility for two exact contracts:

- collector `0.9.0` with `llm-capability-doctor.evidence.v1`: retain the existing
  onsite, full, and custom profile validation so historical logs remain auditable;
- collector `0.10.0` with `llm-capability-doctor.evidence.v2`: reject
  `collection_profile` when present and require the complete 46-test manifest set.

The parser must reject mixed version/schema pairs. The normalized parsed-evidence
and assessment schemas remain unchanged because their `run` object is open-ended
and does not require `collection_profile`.

The report skill instructions and evaluation rules describe both accepted input
contracts. New report conclusions evaluate the four fixed concurrency waves and do
not refer to an onsite profile.

## Documentation

README examples use one default command with no selection flag. The option table
removes `--full` and `--only`, the workload warning states that every run includes
all long-context probes and the 4/8/16/32 concurrency ladder, and release binary
examples use version `0.10.0`.

## Testing

Use test-driven changes in this order:

1. CLI tests require default full selection semantics and reject the removed flags.
2. Catalog and check-plan tests require all 46 checks and all four concurrency
   batches without an onsite branch.
3. Evidence tests require 46 manifests and no `collection_profile` in v2 logs.
4. Parser and skill tests accept historical v1 logs, accept profile-free v2 logs,
   and reject mixed contracts or v2 logs containing the removed field.
5. README and shell tests verify that neither collector advertises or accepts the
   removed options.

Final verification runs Rust formatting, Clippy with warnings denied, all Rust and
Python tests, the locked release build, and searches user-facing help and docs for
stale profile language.

## Non-Goals

This change does not remove any of the 46 checks, renumber test IDs, change probe
semantics other than eliminating the onsite concurrency branch, or introduce a new
way to select a subset of checks.
