# Anthropic Budget and Report Metadata Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Raise every Anthropic request ceiling to 2048 and show URL, requested model, and a collector-masked API-key identifier above Skill-generated report tables.

**Architecture:** Keep credential masking in the Bash collector so complete credentials never enter artifacts. Preserve the existing `run` metadata flow through parsing and assessment, then render it as an escaped definition list in the offline HTML Skill.

**Tech Stack:** Bash, Python 3 standard library, `unittest`, offline HTML/CSS, Codex Skill validation.

---

### Task 1: Add failing collector regression tests

**Files:**
- Modify: `tests/helpers/fake_model_curl.py`
- Modify: `tests/test_model_capability_doctor_script.py`

- [ ] Add `anthropic_message()` and `anthropic_tool_use()` fixture helpers that emit valid Anthropic envelopes.
- [ ] Add an `anthropic_output_budget` scenario that rejects probes without `max_tokens`, returns exact text for tests `004`, `031`, and `032`, and returns a formal `get_weather` call for `040`.
- [ ] Add `test_anthropic_requests_share_the_2048_output_budget`, parse request bodies for `protocol-3`, `test-004`, `test-031`, `test-032`, and `test-040`, and require integer `2048`; also require Thinking `budget_tokens` to remain `1024`.
- [ ] Add `test_run_header_masks_api_key_without_logging_the_complete_value`, requiring `fixture-key` to become `fixt********-key` and requiring the complete key to be absent.
- [ ] Add a short-key test that runs with `--api-key short` and requires `api_key: [MASKED]`.
- [ ] Run the three focused tests and verify they fail because version `0.6.0` emits `[REDACTED]`, `64`, and `256`.

Run:

```bash
python3 -m unittest \
  tests.test_model_capability_doctor_script.ModelCapabilityDoctorScriptTests.test_anthropic_requests_share_the_2048_output_budget \
  tests.test_model_capability_doctor_script.ModelCapabilityDoctorScriptTests.test_run_header_masks_api_key_without_logging_the_complete_value \
  tests.test_model_capability_doctor_script.ModelCapabilityDoctorScriptTests.test_short_api_key_is_fully_masked \
  -v
```

### Task 2: Implement collector budget and masking

**Files:**
- Modify: `model-capability-doctor.sh`
- Modify: `tests/test_model_capability_doctor_script.py`

- [ ] Change the declarations to:

```bash
SCRIPT_VERSION="0.6.1"
ANTHROPIC_MAX_TOKENS="2048"
```

- [ ] Add the masking helper:

```bash
mask_api_key() {
  local value="$1"
  if (( ${#value} <= 8 )); then
    printf '%s' '[MASKED]'
  else
    printf '%s********%s' "${value:0:4}" "${value: -4}"
  fi
}
```

- [ ] Write `api_key: $(mask_api_key "$API_KEY")` in the run header while retaining the full key only in the redaction-secret file.
- [ ] Replace Anthropic literals in `protocol_body()`, `core_multi_turn_body()`, and `core_tool_body()` with the numeric `ANTHROPIC_MAX_TOKENS` value.
- [ ] Update the help/version test to require `0.6.1`.
- [ ] Run the focused collector tests and verify they pass.

### Task 3: Add failing report and Skill-contract tests

**Files:**
- Modify: `tests/test_model_doctor_html.py`
- Modify: `tests/test_model_doctor_log.py`
- Modify: `tests/test_model_doctor_skill.py`

- [ ] Add an HTML test that sets run metadata to a long escaped URL, `deepseek-v4-pro[1m]`, and `sk-t********5678`, then requires `检测 URL`, `模型名称`, and `API Key` with all values escaped and rendered before `能力域总结`.
- [ ] Add a legacy HTML assertion that `[REDACTED]` still renders for `minimal.log`.
- [ ] Add a parser test that changes only the run-header `api_key` to `sk-t********5678` and requires the parsed run value to survive unchanged.
- [ ] Extend the Skill contract test to require the phrase `no unmasked credential values appear`.
- [ ] Run these focused tests and verify they fail because the renderer omits run metadata and the Skill still forbids credential values without distinguishing safe masked identifiers.

Run:

```bash
python3 -m unittest \
  tests.test_model_doctor_html.ModelDoctorHtmlTests.test_report_header_displays_run_metadata \
  tests.test_model_doctor_log.ModelDoctorLogTests.test_parse_log_preserves_collector_masked_api_key \
  tests.test_model_doctor_skill.SkillContractTests.test_skill_declares_evidence_first_workflow \
  -v
```

### Task 4: Implement report metadata rendering and Skill safety text

**Files:**
- Modify: `skills/creating-model-doctor-reports/scripts/model_doctor_html.py`
- Modify: `skills/creating-model-doctor-reports/scripts/model_doctor_assessment.py`
- Modify: `skills/creating-model-doctor-reports/assets/report.css`
- Modify: `skills/creating-model-doctor-reports/SKILL.md`
- Modify: `skills/creating-model-doctor-reports/references/evaluation-rules.md`

- [ ] Add `_run_metadata(run)` that returns an escaped `<section>` and `<dl>` with exactly three labels and fallback value `未知`.
- [ ] Render the metadata block immediately inside `<main>` before the summary table.
- [ ] Add responsive `.run-metadata` styles with wrapping values and stacked mobile rows; do not add a floating card.
- [ ] Update Skill verification to allow only the collector-provided masked identifier and require that no unmasked credential values appear.
- [ ] Register `0.6.1` with the existing 62-item gate mapping and document `0.6.0` as a historical version with the same mapping.
- [ ] Run the focused report tests and verify they pass.

### Task 5: Verify, commit, integrate, and install from repository source

**Files:**
- Modify: all files above
- Create: `docs/superpowers/plans/2026-07-16-anthropic-budget-and-report-metadata.md`

- [ ] Run `bash -n model-capability-doctor.sh`.
- [ ] Run `python3 -m py_compile skills/creating-model-doctor-reports/scripts/*.py`.
- [ ] Run `node --check skills/creating-model-doctor-reports/assets/report.js`.
- [ ] Run the Skill validator against `skills/creating-model-doctor-reports`.
- [ ] Run `python3 -m unittest discover -s tests` and require all tests to pass.
- [ ] Render a fixture report and verify it has no external resources, contains the three metadata values, and contains no complete synthetic key.
- [ ] Run `git diff --check`, inspect only approved files, and commit with `fix: raise Anthropic budget and show report metadata`.
- [ ] Integrate the branch into the main workspace without touching user-generated logs.
- [ ] Back up the stale installed Skill directory, replace it with a symlink to the verified repository Skill, and rerun quick validation through the installed path.
- [ ] Rerun the DeepSeek command to create a new `0.6.1` log before generating the final assessment and HTML; do not overwrite the existing `0.6.0` log or any existing report.
