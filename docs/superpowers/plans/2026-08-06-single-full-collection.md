# Single Full Collection Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove selectable collection modes so the Rust collector always runs all 46 checks, emits a profile-free evidence-v2 log, and remains reportable alongside historical v0.9 evidence-v1 logs.

**Architecture:** Collapse the Rust CLI and runner onto one `catalog::all()` path and one fixed 4/8/16/32 concurrency plan. First teach the report parser the exact v1 and v2 schema/version pairs, then switch the collector to 0.10.0/evidence.v2 so old v1 logs keep their legacy profile validation while new v2 logs reject the deleted field and require all 46 manifests.

**Tech Stack:** Rust 2024, Clap, Tokio, Python 3 `unittest`, Bash, Cargo.

---

### Task 1: Remove Rust CLI selection flags and default to the full catalog

**Files:**
- Modify: `tests/cli.rs`
- Modify: `src/cli.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: Write failing CLI tests**

Replace the profile-oriented CLI tests with assertions that the help omits the deleted options and both old flags are rejected:

```rust
#[test]
fn help_exposes_one_full_collection_workflow() {
    let output = command().arg("--help").output().unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("Run all 46 checks"));
    assert!(!text.contains("--full"));
    assert!(!text.contains("--only"));
}

#[test]
fn removed_selection_flags_are_rejected() {
    for arguments in [vec!["--full"], vec!["--only", "001"]] {
        command().args(arguments).assert().code(2);
    }
}
```

- [ ] **Step 2: Run the focused tests and verify RED**

Run:

```bash
cargo test --test cli --locked
```

Expected: FAIL because help still exposes `--full`/`--only` and the old flags parse successfully.

- [ ] **Step 3: Remove the public flags and selection parser**

In `src/cli.rs`, delete `Cli.only`, `Cli.full`, and `validate_only_syntax`. Keep the internal profile fields for this one intermediate commit, but make every parsed command select full explicitly:

```rust
Ok(Config {
    url,
    model,
    api_key: SecretString(api_key),
    log_file: self.log_file,
    timeout: Duration::from_secs(self.timeout),
    only: None,
    profile: CollectionProfile::Full,
    insecure: self.insecure,
})
```

Change the Clap `about` text to state that the collector runs all 46 checks. In `src/main.rs`, remove `--only` from `OPTIONS_WITH_VALUES` and change its declared length from six to five.

- [ ] **Step 4: Update tests that constructed profile-bearing configs**

Remove tests for custom ordering, malformed selected IDs, duplicate selected IDs, and `--full`/`--only` conflicts. Replace the profile test with one assertion that an ordinary command produces `CollectionProfile::Full` and `config.only == None`.

- [ ] **Step 5: Run focused tests and verify GREEN**

Run:

```bash
cargo test --test cli --locked
```

Expected: PASS with no CLI or catalog selection test failures.

- [ ] **Step 6: Commit the CLI collapse**

```bash
git add src/cli.rs src/main.rs tests/cli.rs
git commit -m "feat: make full collection the only Rust CLI workflow"
```

### Task 2: Teach the report skill both exact audit contracts

**Files:**
- Modify: `tests/test_model_doctor_log.py`
- Modify: `skills/creating-model-doctor-reports/tests/test_model_doctor_v5.py`
- Modify: `skills/creating-model-doctor-reports/scripts/model_doctor_log.py`
- Modify: `skills/creating-model-doctor-reports/SKILL.md`
- Modify: `skills/creating-model-doctor-reports/references/evaluation-rules.md`
- Modify: `tests/test_model_doctor_skill.py`

- [ ] **Step 1: Write failing v1/v2 contract-routing tests**

Import `RETAINED_TEST_IDS` from `model_doctor_log`. Add `write_text` as a method on `ModelDoctorLogTests`:

```python
def write_text(self, value, name):
    directory = tempfile.TemporaryDirectory()
    self.addCleanup(directory.cleanup)
    path = Path(directory.name) / name
    path.write_text(value, encoding="utf-8")
    return path
```

Add `full_v2_log` as a module-level helper that builds a request-free complete v2 log:

```python
def full_v2_log():
    test_ids = sorted(RETAINED_TEST_IDS)
    manifests = "".join(
        "========== TEST-{0} BEGIN ==========\n"
        "name: fixture-{0}\n"
        "category: fixture\n"
        "request_refs: \n"
        "========== TEST-{0} END ==========\n".format(test_id)
        for test_id in test_ids
    )
    return (
        "========== MODEL DOCTOR RUN ==========\n"
        "script_version: 0.10.0\n"
        "section_encoding: base64\n"
        "log_schema: llm-capability-doctor.evidence.v2\n"
        "selected_test_count: 46\n"
        + manifests
        + "========== RUN SUMMARY ==========\n"
        "request_count: 0\n"
        "test_manifest_count: 46\n"
        "========== END ==========\n"
    )
```

Add tests that:

```python
def test_parser_accepts_profile_free_complete_v2(self):
    path = self.write_text(full_v2_log(), "full-v2.log")
    parsed = parse_log(path)
    self.assertEqual(parsed["run"]["log_schema"], "llm-capability-doctor.evidence.v2")
    self.assertNotIn("collection_profile", parsed["run"])
    self.assertEqual(len(parsed["tests"]), 46)

def test_parser_rejects_v2_profile_field(self):
    value = full_v2_log().replace(
        "section_encoding: base64\n",
        "section_encoding: base64\ncollection_profile: full\n",
    )
    path = self.write_text(value, "v2-with-profile.log")
    with self.assertRaisesRegex(ValueError, "must not contain collection_profile"):
        parse_log(path)

def test_parser_rejects_mixed_schema_version_pair(self):
    value = full_v2_log().replace("script_version: 0.10.0", "script_version: 0.9.0")
    path = self.write_text(value, "mixed-contract.log")
    with self.assertRaisesRegex(ValueError, "schema/version pair"):
        parse_log(path)
```

Keep the existing v1 fixture tests, including legacy onsite/full/custom profile validation. Add a fourth v2 test that deletes one manifest and changes both declared counts to 45; it must still fail with `Evidence v2 must contain all 46 retained tests`.

- [ ] **Step 2: Run parser tests and verify RED**

Run:

```bash
python3 -m unittest tests.test_model_doctor_log -v
```

Expected: FAIL because the parser only accepts collector 0.9.0/evidence.v1.

- [ ] **Step 3: Route parsing by exact contract pair**

Replace the single schema/version constants in `model_doctor_log.py` with:

```python
V1_CONTRACT = ("llm-capability-doctor.evidence.v1", "0.9.0")
V2_CONTRACT = ("llm-capability-doctor.evidence.v2", "0.10.0")
SUPPORTED_CONTRACTS = {V1_CONTRACT, V2_CONTRACT}
```

At the start of `parse_log`, validate the pair together. Build the tuple in schema/version order so it matches the constants:

```python
contract = (raw_run.get("log_schema"), raw_run.get("script_version"))
if contract not in SUPPORTED_CONTRACTS:
    raise ValueError(f"Unsupported log schema/version pair: {contract!r}")
```

After manifests are parsed, preserve the existing profile rules only under `V1_CONTRACT`. Under `V2_CONTRACT`, enforce:

```python
if "collection_profile" in run:
    raise ValueError("Evidence v2 must not contain collection_profile")
if discovered_test_ids != RETAINED_TEST_IDS:
    raise ValueError("Evidence v2 must contain all 46 retained tests")
```

Keep section encoding, count, duplicate-block, request-reference, redaction, and retained-ID validation unchanged.

- [ ] **Step 4: Update the skill contract and concurrency rule**

Change `SKILL.md` metadata and Input Contract to accept exactly v0.9/evidence.v1 and v0.10/evidence.v2, state that v1 retains profile validation, and state that v2 is profile-free and complete.

Change the evidence-scope rule to evaluate all retained manifests for accepted contracts. Replace the check-057 method with:

```markdown
- Method: inspect the 4、8、16、32 concurrent waves.
```

Update `tests/test_model_doctor_skill.py` so required contract strings include both versions and both schemas. Add focused contract validation in `test_model_doctor_v5.py`; keep its historical v1 fixture cases unchanged.

- [ ] **Step 5: Verify parser and skill tests GREEN**

Run:

```bash
python3 -m unittest tests.test_model_doctor_log tests.test_model_doctor_skill -v
python3 -m unittest discover -s skills/creating-model-doctor-reports/tests -p 'test_model_doctor_v5.py' -v
```

Expected: PASS for historical v1 parsing, new profile-free v2 parsing, missing-manifest rejection, and mixed-pair rejection. The collector still emits v1 at this checkpoint, so no Rust-generated v2 test is added yet.

- [ ] **Step 6: Commit the dual-contract report support**

```bash
git add tests/test_model_doctor_log.py tests/test_model_doctor_skill.py skills/creating-model-doctor-reports/SKILL.md skills/creating-model-doctor-reports/references/evaluation-rules.md skills/creating-model-doctor-reports/scripts/model_doctor_log.py skills/creating-model-doctor-reports/tests/test_model_doctor_v5.py
git commit -m "feat: parse profile-free evidence v2 logs"
```

### Task 3: Remove the onsite execution branch and emit evidence v2

**Files:**
- Modify: `tests/check_contracts.rs`
- Modify: `src/checks/mod.rs`
- Modify: `src/checks/performance.rs`
- Modify: `tests/catalog.rs`
- Modify: `src/catalog.rs`
- Modify: `tests/evidence.rs`
- Modify: `tests/signals.rs`
- Modify: `tests/cli.rs`
- Modify: `src/audit.rs`
- Modify: `src/runner.rs`
- Modify: `src/cli.rs`
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`

- [ ] **Step 1: Write failing fixed-concurrency and v2 audit tests**

Remove the `onsite_context` helper and onsite-only concurrency test. Construct `PlanContext` without an `onsite` field:

```rust
fn context(protocol: Protocol) -> PlanContext<'static> {
    PlanContext {
        protocol,
        auth_mode: AuthMode::Bearer,
        model: "fixture-model",
    }
}
```

Keep `concurrency_check_has_four_simultaneous_batches_and_sixty_unique_ids` as the only check-057 contract. Replace the catalog selection tests with:

```rust
#[test]
fn all_returns_the_complete_ordered_catalog() {
    let selected = all();
    assert_eq!(selected.len(), 46);
    assert_eq!(selected.first().unwrap().id, "001");
    assert_eq!(selected.last().unwrap().id, "060");
}
```

Change audit assertions to require the new header and the absence of the removed field:

```rust
assert!(log.contains("script_version: 0.10.0"));
assert!(log.contains("log_schema: llm-capability-doctor.evidence.v2"));
assert!(!log.contains("collection_profile:"));
```

Change signal tests to wait for and assert `llm-capability-doctor.evidence.v2`, removing `--only 004` from spawned binary arguments. Rename `parser_compatibility_accepts_rust_generated_evidence_v1` to `parser_compatibility_accepts_rust_generated_evidence_v2`; assert the parser returns schema v2 and no `collection_profile`.

- [ ] **Step 2: Run focused tests and verify RED**

Run:

```bash
cargo test --test catalog --test check_contracts --test evidence --test signals --locked
```

Expected: FAIL because `PlanContext` still requires `onsite`, `catalog::all` does not exist, audit logs still emit v0.9/v1 plus `collection_profile`, and binary fixtures still rely on `--only`.

- [ ] **Step 3: Make the catalog and concurrency plan unconditional**

Delete `PlanContext.onsite` from `src/checks/mod.rs`. Replace the check-057 branch in `src/checks/performance.rs` with the fixed ladder:

```rust
let groups = [4_usize, 8, 16, 32]
    .into_iter()
    .map(|concurrency| {
        let requests = (1..=concurrency)
            .map(|index| {
                basic(
                    &format!("test-057-c{concurrency}-{index}"),
                    &format!("Reply only MODEL_DOCTOR_057_C{concurrency}_OK"),
                    false,
                    context,
                )
            })
            .collect();
        RequestGroup::Concurrent(requests)
    })
    .collect();
```

Remove `onsite` from the runner's `PlanContext` construction. Delete `HashSet`, `ONSITE_TEST_IDS`, `CatalogError`, `select_onsite`, and `select` from `src/catalog.rs`, then add:

```rust
pub fn all() -> Vec<&'static TestCase> {
    CATALOG.iter().collect()
}
```

Remove `Config.only`, `Config.profile`, `CollectionProfile`, and its `Display` implementation from `src/cli.rs`. Remove the intermediate `CollectionProfile::Full` assertion and import from `tests/cli.rs`; ordinary parsed commands should now be asserted only through their endpoint, timeout, log, and TLS fields. In `src/runner.rs`, remove `CatalogError`, `CollectionProfile`, and `select_onsite`, initialize `selected` with `crate::catalog::all()`, and delete the `Catalog` error variant. Do not add a replacement selection hook.

- [ ] **Step 4: Version and simplify the audit header**

Set the package version in `Cargo.toml` to `0.10.0` and update `src/cli.rs` about text to `Model Capability Doctor 0.10.0`.

Remove `RunMetadata.collection_profile` and its import from `src/audit.rs`. Write this exact header contract:

```rust
writeln!(self.writer, "script_version: 0.10.0")?;
writeln!(self.writer, "collector_runtime: rust")?;
writeln!(self.writer, "section_encoding: base64")?;
writeln!(self.writer, "log_schema: llm-capability-doctor.evidence.v2")?;
```

Remove `collection_profile` from `RunMetadata` construction in `src/runner.rs` and all test fixtures. Run `cargo check --locked`; if the lockfile version mismatch is reported, run `cargo check` once to update only the root package entry in `Cargo.lock`.

- [ ] **Step 5: Convert Rust end-to-end fixtures to full runs**

Change `run_fixture(protocol, only)` in `tests/evidence.rs` to `run_fixture(protocol)` and remove `--only` from its command. Delete the duplicate `run_default_fixture`; update callers to inspect the relevant manifest inside the full 46-manifest log.

Change assertions that expected a selected manifest count to:

```rust
assert!(log.contains("selected_test_count: 46"));
assert!(log.contains("test_manifest_count: 46"));
assert!(!log.contains("collection_profile:"));
```

Keep the existing request-reference assertions for checks 002, 003, 007, 033, 047-049, 055-057, 059, and 060. The transport and TLS tests also run the complete catalog and inspect only their relevant transport evidence. The renamed parser-compatibility test now consumes a genuine Rust v2 full log, which Task 2's parser already accepts.

- [ ] **Step 6: Run Rust tests and verify GREEN**

Run:

```bash
cargo fmt --check
cargo test --all-targets --all-features --locked
```

Expected: PASS; evidence logs declare v2, contain 46 manifests, contain 60 check-057 request refs, omit `collection_profile`, and parse successfully through the report skill.

- [ ] **Step 7: Commit the single execution plan and v2 writer**

```bash
git add Cargo.toml Cargo.lock src/audit.rs src/catalog.rs src/checks/mod.rs src/checks/performance.rs src/cli.rs src/runner.rs tests/catalog.rs tests/check_contracts.rs tests/evidence.rs tests/signals.rs tests/cli.rs
git commit -m "feat: emit profile-free full evidence logs"
```

### Task 4: Remove legacy Shell selection and rewrite user documentation

**Files:**
- Modify: `tests/test_model_capability_doctor_script.py`
- Modify: `model-capability-doctor.sh`
- Modify: `tests/cli.rs`
- Modify: `README.md`

- [ ] **Step 1: Write failing Shell and README tests**

Add a Shell CLI rejection test:

```python
def test_only_option_is_removed(self):
    result = self.run_script("--only", "004")
    self.assertEqual(result.returncode, 2)
    self.assertIn("Unknown option: --only", result.stderr)
```

Change the Rust README test to require `0.10.0`, `llm-capability-doctor.evidence.v2`, and the phrase `默认执行全部 46 个检测项`, while asserting that `--full`, `--only`, `默认现场模式`, and `collection_profile` are absent.

- [ ] **Step 2: Run documentation and Shell tests and verify RED**

Run:

```bash
python3 -m unittest tests.test_model_capability_doctor_script -v
cargo test --test cli documentation_publishes_the_rust_cli_workflow_and_security_warning --locked
```

Expected: FAIL because the Shell still accepts `--only` and README still documents three collection modes.

- [ ] **Step 3: Remove Shell selection code**

Delete the `--only` usage line, `ONLY_IDS`, `catalog_row`, `selected_catalog`, `validate_only_ids`, the `--only` case arm, and the validation call. Where the script iterates selected tests, replace `selected_catalog` with `print_core_catalog`. Set `SELECTED_TEST_COUNT` from the complete catalog:

```bash
SELECTED_TEST_COUNT="$(print_core_catalog | wc -l | tr -d ' ')"
```

Update the Shell test fixture helper to drop its `only` parameter and stop passing `--only`; every fixture run executes all 62 legacy Shell checks. Change `test_manifest_count` expectations to 62 while keeping scenario-specific assertions scoped to the relevant test blocks. Remove `--only` from the TERM-signal process arguments, remove it from the missing-value option list, and delete tests whose only purpose was validating selected-ID syntax.

- [ ] **Step 4: Rewrite README around one command**

Update release filenames and build examples from `0.9.0` to `0.10.0`. Keep one run example without a selection flag. Replace the modes table with a statement that every Rust run executes all 46 checks. State that the workload always includes all five long-context probes and all four concurrency waves, and label the log path as evidence-v2.

Remove every user-facing occurrence of `--full`, `--only`, onsite/现场模式, custom selection, and `collection_profile`. Keep `--list-tests`, TLS guidance, security handling, and the legacy Shell warning.

- [ ] **Step 5: Run Shell and documentation tests and verify GREEN**

Run:

```bash
python3 -m unittest tests.test_model_capability_doctor_script -v
cargo test --test cli --locked
! rg -n -- '--full|--only|collection_profile|默认现场模式|onsite profile' README.md src model-capability-doctor.sh
```

Expected: tests PASS and `rg` returns no user-facing production matches; historical fixtures, compatibility tests, and superseded design documents may still contain legacy terms.

- [ ] **Step 6: Commit Shell and documentation changes**

```bash
git add README.md model-capability-doctor.sh tests/cli.rs tests/test_model_capability_doctor_script.py
git commit -m "docs: publish one full collection workflow"
```

### Task 5: Run release verification

**Files:**
- Verify: all modified files

- [ ] **Step 1: Run formatting and static analysis**

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: both commands exit 0 with no warnings.

- [ ] **Step 2: Run the complete automated test suite**

```bash
cargo test --all-targets --all-features --locked
python3 -m unittest discover -s tests -p 'test_*.py' -v
python3 -m unittest discover -s skills/creating-model-doctor-reports/tests -p 'test_*.py' -v
```

Expected: all Rust, repository Python, and skill Python tests PASS.

- [ ] **Step 3: Build the locked release binary**

```bash
cargo build --release --locked
./target/release/model-capability-doctor --version
./target/release/model-capability-doctor --help
```

Expected: build exits 0, version prints `0.10.0`, and help contains neither `--full` nor `--only`.

- [ ] **Step 4: Verify the final repository diff and commit state**

```bash
git status --short
git log -5 --oneline
```

Expected: only the user's pre-existing untracked report files remain; implementation files are committed and the recent commits correspond to the four tasks above.
