# Model Doctor Report Verified Capability Facts Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make every newly generated Model Doctor report include evidence-bounded interface protocol, context-window, and concurrency facts in its final conclusion.

**Architecture:** Add a focused verified-facts validator shared by review and assessment validation. Upgrade the authored review envelope to `reviews.v2` and rendered assessment to `assessment.v6`, carry the required facts inside `capabilitySummary.verifiedFacts`, and render them as a compact definition list before the existing headline and failure issues.

**Tech Stack:** Python 3.9 standard library, JSON Schema draft 2020-12, `unittest`, self-contained HTML/CSS.

---

## File Structure

- Create `skills/creating-model-doctor-reports/scripts/model_doctor_verified_facts.py`: own fact enums, field sets, cross-field state rules, evidence-domain validation, and bounded-language checks.
- Rename `skills/creating-model-doctor-reports/tests/test_model_doctor_v5.py` to `skills/creating-model-doctor-reports/tests/test_model_doctor_v6.py`: keep the tracked report regression suite aligned with the current contract and add fact-specific tests.
- Modify `skills/creating-model-doctor-reports/scripts/model_doctor_assessment.py`: adopt reviews.v2/assessment.v6, build allowed fact-evidence domains, call the shared validator, and preserve facts during assembly.
- Modify `skills/creating-model-doctor-reports/references/assessment-schema.json`: declare assessment.v6 and the required `verifiedFacts` definitions.
- Modify `skills/creating-model-doctor-reports/scripts/model_doctor_html.py`: render the three facts inside “最终结论”.
- Modify `skills/creating-model-doctor-reports/assets/report.css`: style the fact definition list for screen, mobile, and print.
- Modify `skills/creating-model-doctor-reports/SKILL.md`: require the new evidence-synthesis stage and reviews.v2 envelope.
- Modify `skills/creating-model-doctor-reports/references/evaluation-rules.md`: define evidence sources, state handling, and no-hard-limit language.

### Task 1: Add the Verified-Facts Review Contract

**Files:**
- Create: `skills/creating-model-doctor-reports/scripts/model_doctor_verified_facts.py`
- Rename: `skills/creating-model-doctor-reports/tests/test_model_doctor_v5.py` to `skills/creating-model-doctor-reports/tests/test_model_doctor_v6.py`
- Modify: `skills/creating-model-doctor-reports/scripts/model_doctor_assessment.py:13-64,189-458`
- Test: `skills/creating-model-doctor-reports/tests/test_model_doctor_v6.py`

- [ ] **Step 1: Rename the tracked test module and add a complete reviews.v2 fixture**

Run:

```bash
git mv skills/creating-model-doctor-reports/tests/test_model_doctor_v5.py \
  skills/creating-model-doctor-reports/tests/test_model_doctor_v6.py
```

Add this helper to `ModelDoctorV6Tests` and make `_summary()` include `"verifiedFacts": self._verified_facts()`:

```python
def _verified_facts(self) -> dict:
    return {
        "interfaceProtocol": {
            "evidenceState": "INCONCLUSIVE",
            "family": "UNKNOWN",
            "requestFormat": "已采集请求，但未形成已知协议结论。",
            "responseFormat": "已采集响应，但未形成已知协议结论。",
            "statement": "本轮证据不足以确认接口协议格式。",
            "evidenceRefs": ["request:req-fail"],
            "boundary": "不能从 URL 或模型名称猜测协议格式。",
        },
        "contextWindow": {
            "evidenceState": "NOT_COLLECTED",
            "highestVerifiedTier": None,
            "highestVerifiedInputTokens": None,
            "firstFailedTier": None,
            "firstFailedInputTokens": None,
            "statement": "本轮未采集上下文档位证据。",
            "evidenceRefs": [],
            "boundary": "未采集时不能推断上下文上限。",
        },
        "concurrency": {
            "evidenceState": "NOT_COLLECTED",
            "highestVerifiedConcurrentRequests": None,
            "statement": "本轮未采集并发波次证据。",
            "evidenceRefs": [],
            "boundary": "未采集时不能推断并发上限。",
        },
    }
```

Change fixture schema strings to `llm-capability-doctor.reviews.v2` and rename the test class to `ModelDoctorV6Tests`.

- [ ] **Step 2: Write failing validation tests**

Add tests that require the new object and exercise the state rules:

```python
def test_reviews_v2_requires_verified_facts(self) -> None:
    reviews = self._reviews()
    del reviews["capabilitySummary"]["verifiedFacts"]
    self.assertIn(
        "capabilitySummary verifiedFacts must be an object",
        validate_reviews(self._parsed(), reviews),
    )

def test_verified_context_rejects_unbounded_max_claim(self) -> None:
    reviews = self._reviews()
    context = reviews["capabilitySummary"]["verifiedFacts"]["contextWindow"]
    context.update(
        evidenceState="VERIFIED",
        highestVerifiedTier="32K Token 近似档",
        highestVerifiedInputTokens=80175,
        statement="最大上下文是 80175 Token。",
        evidenceRefs=["request:req-fail"],
    )
    errors = validate_reviews(self._parsed(), reviews)
    self.assertTrue(any("unbounded maximum claim" in error for error in errors))

def test_not_collected_rejects_values_and_evidence(self) -> None:
    reviews = self._reviews()
    concurrency = reviews["capabilitySummary"]["verifiedFacts"]["concurrency"]
    concurrency["highestVerifiedConcurrentRequests"] = 32
    concurrency["evidenceRefs"] = ["request:req-fail"]
    errors = validate_reviews(self._parsed(), reviews)
    self.assertTrue(any("NOT_COLLECTED" in error for error in errors))
```

- [ ] **Step 3: Run the focused tests and confirm RED**

Run:

```bash
PYTHONPATH=skills/creating-model-doctor-reports/tests python3 -m unittest \
  test_model_doctor_v6.ModelDoctorV6Tests.test_reviews_v2_requires_verified_facts \
  test_model_doctor_v6.ModelDoctorV6Tests.test_verified_context_rejects_unbounded_max_claim \
  test_model_doctor_v6.ModelDoctorV6Tests.test_not_collected_rejects_values_and_evidence -v
```

Expected: FAIL because reviews.v2 and `verifiedFacts` are not implemented.

- [ ] **Step 4: Create the shared validator**

Create `model_doctor_verified_facts.py` with these public contracts and exact field sets:

```python
from __future__ import annotations

import re
from typing import Dict, List, Set

EVIDENCE_STATES = {"VERIFIED", "INCONCLUSIVE", "NOT_COLLECTED"}
PROTOCOL_FAMILIES = {
    "OPENAI_CHAT_COMPLETIONS",
    "OPENAI_RESPONSES",
    "ANTHROPIC_MESSAGES",
    "GEMINI_GENERATE_CONTENT",
    "OLLAMA_CHAT",
    "CUSTOM",
    "UNKNOWN",
}
VERIFIED_FACTS_FIELDS = {"interfaceProtocol", "contextWindow", "concurrency"}
INTERFACE_FIELDS = {
    "evidenceState", "family", "requestFormat", "responseFormat",
    "statement", "evidenceRefs", "boundary",
}
CONTEXT_FIELDS = {
    "evidenceState", "highestVerifiedTier", "highestVerifiedInputTokens",
    "firstFailedTier", "firstFailedInputTokens", "statement",
    "evidenceRefs", "boundary",
}
CONCURRENCY_FIELDS = {
    "evidenceState", "highestVerifiedConcurrentRequests", "statement",
    "evidenceRefs", "boundary",
}
_UNBOUNDED_CLAIMS = re.compile(
    r"真实最大|硬上限|最大上下文|最大并发|一定支持更高"
)


def _non_empty(value: object) -> bool:
    return isinstance(value, str) and bool(value.strip())


def _refs(value: object) -> bool:
    return (
        isinstance(value, list)
        and all(_non_empty(item) for item in value)
        and len(value) == len(set(value))
    )


def _unbounded_claim(value: object) -> bool:
    if not isinstance(value, str):
        return False
    safe = (
        "不是硬上限", "不代表硬上限", "真实上限未测试",
        "上限无法确认", "不能确定上限",
    )
    reduced = value
    for phrase in safe:
        reduced = reduced.replace(phrase, "")
    return bool(_UNBOUNDED_CLAIMS.search(reduced))


def validate_verified_facts(
    facts: object,
    evidence_domains: Dict[str, Set[str]],
) -> List[str]:
    """Validate required fact shapes, state/value combinations, and evidence domains."""
    errors: List[str] = []
    if not isinstance(facts, dict):
        return ["capabilitySummary verifiedFacts must be an object"]
    for field in sorted(set(facts) - VERIFIED_FACTS_FIELDS):
        errors.append(f"verifiedFacts field {field} is not allowed")
    for field in sorted(VERIFIED_FACTS_FIELDS - set(facts)):
        errors.append(f"verifiedFacts {field} is required")
    if errors:
        return errors

    errors.extend(_validate_interface(facts["interfaceProtocol"], evidence_domains["interfaceProtocol"]))
    errors.extend(_validate_context(facts["contextWindow"], evidence_domains["contextWindow"]))
    errors.extend(_validate_concurrency(facts["concurrency"], evidence_domains["concurrency"]))
    return errors
```

Implement `_validate_interface`, `_validate_context`, and `_validate_concurrency` with these exact relational rules:

```python
# Shared rules for every object:
# - value must be an object with exactly its declared field set;
# - evidenceState must be in EVIDENCE_STATES;
# - statement and boundary must be non-empty strings;
# - evidenceRefs must be a unique string array and a subset of its allowed domain;
# - statement must not satisfy _unbounded_claim().

# interfaceProtocol:
# - family must be in PROTOCOL_FAMILIES;
# - requestFormat and responseFormat are always non-empty;
# - VERIFIED requires non-empty refs and family != UNKNOWN;
# - INCONCLUSIVE requires non-empty refs;
# - NOT_COLLECTED requires family == UNKNOWN and empty refs.

# contextWindow:
# - tiers are non-empty strings or None;
# - token values are non-negative integers or None, with bool explicitly rejected;
# - firstFailedInputTokens must be None when firstFailedTier is None;
# - VERIFIED requires highestVerifiedTier and non-empty refs;
# - INCONCLUSIVE requires non-empty refs;
# - NOT_COLLECTED requires all tier/token values None and refs empty.

# concurrency:
# - highestVerifiedConcurrentRequests is a positive integer or None, with bool rejected;
# - VERIFIED requires a positive value and non-empty refs;
# - INCONCLUSIVE requires non-empty refs;
# - NOT_COLLECTED requires value None and refs empty.
```

Prefix every error with `interfaceProtocol`, `contextWindow`, or `concurrency` so failures identify the exact location.

- [ ] **Step 5: Integrate reviews.v2 validation**

In `model_doctor_assessment.py`:

```python
from model_doctor_verified_facts import validate_verified_facts

REVIEW_SCHEMA_VERSION = "llm-capability-doctor.reviews.v2"
ASSESSMENT_SCHEMA_VERSION = "llm-capability-doctor.assessment.v6"
CAPABILITY_SUMMARY_FIELDS = {
    "headline", "verifiedFacts", "issues", "scopeBoundary"
}


def _parsed_fact_evidence_domains(parsed: dict) -> Dict[str, set[str]]:
    tests = parsed.get("tests", {})

    def request_refs(test_ids: set[str]) -> set[str]:
        return {
            f"request:{request_id}"
            for test_id in test_ids
            for request_id in tests.get(test_id, {}).get("requestRefs", [])
        }

    return {
        "interfaceProtocol": request_refs({"002"}),
        "contextWindow": request_refs({"014", "015", "016", "017", "018"}),
        "concurrency": request_refs({"057"}),
    }
```

After validating summary headline and scope boundary, add:

```python
errors.extend(
    validate_verified_facts(
        summary.get("verifiedFacts"),
        _parsed_fact_evidence_domains(parsed),
    )
)
```

- [ ] **Step 6: Run the focused tests and full tracked Skill suite**

Run:

```bash
python3 -m unittest discover \
  -s skills/creating-model-doctor-reports/tests \
  -p 'test_*.py' -v
```

Expected: all tests pass with reviews.v2 fixture data.

- [ ] **Step 7: Commit Task 1**

```bash
git add skills/creating-model-doctor-reports/scripts/model_doctor_verified_facts.py \
  skills/creating-model-doctor-reports/scripts/model_doctor_assessment.py \
  skills/creating-model-doctor-reports/tests/test_model_doctor_v6.py
git commit -m "feat: require verified capability facts in reviews"
```

### Task 2: Upgrade Assessment Assembly and JSON Schema to v6

**Files:**
- Modify: `skills/creating-model-doctor-reports/scripts/model_doctor_assessment.py:498-803`
- Modify: `skills/creating-model-doctor-reports/references/assessment-schema.json`
- Test: `skills/creating-model-doctor-reports/tests/test_model_doctor_v6.py`

- [ ] **Step 1: Add failing assessment and schema tests**

```python
def test_assembly_emits_assessment_v6_with_verified_facts(self) -> None:
    assessment = assemble_assessment(self._parsed(), self._reviews())
    self.assertEqual("llm-capability-doctor.assessment.v6", assessment["schemaVersion"])
    self.assertEqual(
        self._verified_facts(),
        assessment["capabilitySummary"]["verifiedFacts"],
    )
    self.assertEqual([], validate_assessment(assessment))

def test_assessment_schema_requires_verified_facts(self) -> None:
    schema = json.loads(
        (SKILL_DIR / "references" / "assessment-schema.json").read_text(encoding="utf-8")
    )
    self.assertEqual("llm-capability-doctor.assessment.v6", schema["$id"])
    summary = schema["$defs"]["capabilitySummary"]
    self.assertIn("verifiedFacts", summary["required"])
    self.assertIn("verifiedFacts", schema["$defs"])
```

- [ ] **Step 2: Run tests and confirm RED**

Run:

```bash
PYTHONPATH=skills/creating-model-doctor-reports/tests python3 -m unittest \
  test_model_doctor_v6.ModelDoctorV6Tests.test_assembly_emits_assessment_v6_with_verified_facts \
  test_model_doctor_v6.ModelDoctorV6Tests.test_assessment_schema_requires_verified_facts -v
```

Expected: schema test fails because the file still declares assessment.v5 and lacks the new definitions.

- [ ] **Step 3: Validate facts in assembled assessments**

Add an assessment-domain adapter and call the shared validator from `_validate_assessment_summary`:

```python
def _assessment_fact_evidence_domains(items: List[dict]) -> Dict[str, set[str]]:
    items_by_id = {item.get("testId"): item for item in items}

    def request_refs(test_ids: set[str]) -> set[str]:
        return {
            f"request:{request.get('request_id')}"
            for test_id in test_ids
            for request in items_by_id.get(test_id, {}).get("requests", [])
            if request.get("request_id")
        }

    return {
        "interfaceProtocol": request_refs({"002"}),
        "contextWindow": request_refs({"014", "015", "016", "017", "018"}),
        "concurrency": request_refs({"057"}),
    }
```

Change `_validate_assessment_summary(items, summary)` to call:

```python
errors.extend(
    validate_verified_facts(
        summary.get("verifiedFacts"),
        _assessment_fact_evidence_domains(items),
    )
)
```

Update obsolete-contract error text from “v5” to “v6”. Assembly already deep-copies `capabilitySummary`, so no separate transformation is needed.

- [ ] **Step 4: Upgrade the JSON Schema**

Change `$id` and `schemaVersion.const` to `llm-capability-doctor.assessment.v6`. Add `verifiedFacts` to `capabilitySummary.required` and add definitions matching the design:

```json
"verifiedFacts": {
  "type": "object",
  "required": ["interfaceProtocol", "contextWindow", "concurrency"],
  "properties": {
    "interfaceProtocol": {"$ref": "#/$defs/interfaceProtocolFact"},
    "contextWindow": {"$ref": "#/$defs/contextWindowFact"},
    "concurrency": {"$ref": "#/$defs/concurrencyFact"}
  },
  "additionalProperties": false
}
```

Add these sibling definitions under `$defs`; the Python validator remains responsible for cross-field state rules and evidence-domain membership:

```json
"interfaceProtocolFact": {
  "type": "object",
  "required": [
    "evidenceState", "family", "requestFormat", "responseFormat",
    "statement", "evidenceRefs", "boundary"
  ],
  "properties": {
    "evidenceState": {
      "enum": ["VERIFIED", "INCONCLUSIVE", "NOT_COLLECTED"]
    },
    "family": {
      "enum": [
        "OPENAI_CHAT_COMPLETIONS", "OPENAI_RESPONSES",
        "ANTHROPIC_MESSAGES", "GEMINI_GENERATE_CONTENT",
        "OLLAMA_CHAT", "CUSTOM", "UNKNOWN"
      ]
    },
    "requestFormat": {"type": "string", "minLength": 1},
    "responseFormat": {"type": "string", "minLength": 1},
    "statement": {"type": "string", "minLength": 1},
    "evidenceRefs": {
      "type": "array",
      "items": {"type": "string", "minLength": 1},
      "uniqueItems": true
    },
    "boundary": {"type": "string", "minLength": 1}
  },
  "additionalProperties": false
},
"contextWindowFact": {
  "type": "object",
  "required": [
    "evidenceState", "highestVerifiedTier", "highestVerifiedInputTokens",
    "firstFailedTier", "firstFailedInputTokens", "statement",
    "evidenceRefs", "boundary"
  ],
  "properties": {
    "evidenceState": {
      "enum": ["VERIFIED", "INCONCLUSIVE", "NOT_COLLECTED"]
    },
    "highestVerifiedTier": {"type": ["string", "null"], "minLength": 1},
    "highestVerifiedInputTokens": {"type": ["integer", "null"], "minimum": 0},
    "firstFailedTier": {"type": ["string", "null"], "minLength": 1},
    "firstFailedInputTokens": {"type": ["integer", "null"], "minimum": 0},
    "statement": {"type": "string", "minLength": 1},
    "evidenceRefs": {
      "type": "array",
      "items": {"type": "string", "minLength": 1},
      "uniqueItems": true
    },
    "boundary": {"type": "string", "minLength": 1}
  },
  "additionalProperties": false
},
"concurrencyFact": {
  "type": "object",
  "required": [
    "evidenceState", "highestVerifiedConcurrentRequests",
    "statement", "evidenceRefs", "boundary"
  ],
  "properties": {
    "evidenceState": {
      "enum": ["VERIFIED", "INCONCLUSIVE", "NOT_COLLECTED"]
    },
    "highestVerifiedConcurrentRequests": {
      "type": ["integer", "null"],
      "minimum": 1
    },
    "statement": {"type": "string", "minLength": 1},
    "evidenceRefs": {
      "type": "array",
      "items": {"type": "string", "minLength": 1},
      "uniqueItems": true
    },
    "boundary": {"type": "string", "minLength": 1}
  },
  "additionalProperties": false
}
```

Reference `$defs.verifiedFacts` from `capabilitySummary.properties.verifiedFacts`.

- [ ] **Step 5: Run the schema, assessment, and full Skill tests**

Run:

```bash
python3 -m unittest discover \
  -s skills/creating-model-doctor-reports/tests \
  -p 'test_*.py' -v
```

Expected: all tests pass and assessment assembly emits v6.

- [ ] **Step 6: Commit Task 2**

```bash
git add skills/creating-model-doctor-reports/scripts/model_doctor_assessment.py \
  skills/creating-model-doctor-reports/references/assessment-schema.json \
  skills/creating-model-doctor-reports/tests/test_model_doctor_v6.py
git commit -m "feat: emit assessment v6 capability facts"
```

### Task 3: Render Facts in the Final Conclusion

**Files:**
- Modify: `skills/creating-model-doctor-reports/scripts/model_doctor_html.py:93-121`
- Modify: `skills/creating-model-doctor-reports/assets/report.css:82-137`
- Test: `skills/creating-model-doctor-reports/tests/test_model_doctor_v6.py`

- [ ] **Step 1: Add failing HTML tests**

```python
def test_final_conclusion_renders_all_three_fact_rows(self) -> None:
    html = render_report(
        assemble_assessment(self._parsed(), self._reviews()),
        ASSET_DIR,
    )
    self.assertIn("接口协议格式", html)
    self.assertIn("本轮证据不足以确认接口协议格式", html)
    self.assertIn("上下文能力", html)
    self.assertIn("本轮未采集上下文档位证据", html)
    self.assertIn("并发能力", html)
    self.assertIn("本轮未采集并发波次证据", html)

def test_verified_facts_escape_dynamic_text(self) -> None:
    reviews = self._reviews()
    reviews["capabilitySummary"]["verifiedFacts"]["interfaceProtocol"]["statement"] = (
        "<script>alert(1)</script>"
    )
    html = render_report(assemble_assessment(self._parsed(), reviews), ASSET_DIR)
    self.assertNotIn("<script>alert(1)</script>", html)
    self.assertIn("&lt;script&gt;alert(1)&lt;/script&gt;", html)
```

- [ ] **Step 2: Run tests and confirm RED**

Run:

```bash
PYTHONPATH=skills/creating-model-doctor-reports/tests python3 -m unittest \
  test_model_doctor_v6.ModelDoctorV6Tests.test_final_conclusion_renders_all_three_fact_rows \
  test_model_doctor_v6.ModelDoctorV6Tests.test_verified_facts_escape_dynamic_text -v
```

Expected: FAIL because no fact markup exists.

- [ ] **Step 3: Add the fact renderer**

Add helpers before `_capability_summary`:

```python
PROTOCOL_LABELS = {
    "OPENAI_CHAT_COMPLETIONS": "OpenAI Chat Completions",
    "OPENAI_RESPONSES": "OpenAI Responses",
    "ANTHROPIC_MESSAGES": "Anthropic Messages",
    "GEMINI_GENERATE_CONTENT": "Gemini GenerateContent",
    "OLLAMA_CHAT": "Ollama Chat",
    "CUSTOM": "自定义格式",
    "UNKNOWN": "未确认",
}


def _verified_facts(facts: dict) -> str:
    protocol = facts.get("interfaceProtocol", {})
    context = facts.get("contextWindow", {})
    concurrency = facts.get("concurrency", {})
    rows = (
        (
            "接口协议格式",
            protocol.get("statement"),
            "请求：{}；响应：{}；分类：{}。".format(
                protocol.get("requestFormat"),
                protocol.get("responseFormat"),
                PROTOCOL_LABELS.get(protocol.get("family"), protocol.get("family")),
            ),
            protocol.get("boundary"),
        ),
        (
            "上下文能力",
            context.get("statement"),
            "最高通过档：{}；原生输入 Token：{}；首个失败档：{}；失败档输入 Token：{}。".format(
                context.get("highestVerifiedTier") or "未确认",
                context.get("highestVerifiedInputTokens") if context.get("highestVerifiedInputTokens") is not None else "未观察到",
                context.get("firstFailedTier") or "未观察到",
                context.get("firstFailedInputTokens") if context.get("firstFailedInputTokens") is not None else "未观察到",
            ),
            context.get("boundary"),
        ),
        (
            "并发能力",
            concurrency.get("statement"),
            "最高已验证并发：{}。".format(
                concurrency.get("highestVerifiedConcurrentRequests")
                if concurrency.get("highestVerifiedConcurrentRequests") is not None
                else "未确认"
            ),
            concurrency.get("boundary"),
        ),
    )
    return '<dl class="verified-facts">' + "".join(
        "<div>"
        f"<dt>{_e(label)}</dt>"
        f"<dd><strong>{_e(statement)}</strong>"
        f'<span class="verified-fact-detail">{_e(detail)}</span>'
        f'<span class="verified-fact-boundary">{_e(boundary)}</span></dd>'
        "</div>"
        for label, statement, detail, boundary in rows
    ) + "</dl>"
```

Insert `f'{_verified_facts(summary.get("verifiedFacts", {}))}'` immediately after the “最终结论” heading and before `final-conclusion-lead`.

- [ ] **Step 4: Add restrained responsive CSS**

```css
.verified-facts {
  margin: 0 0 1rem;
  border-top: 1px solid var(--line);
}

.verified-facts > div {
  display: grid;
  grid-template-columns: 7rem minmax(0, 1fr);
  gap: 0.65rem;
  padding: 0.65rem 0;
  border-bottom: 1px solid var(--line);
}

.verified-facts dt {
  color: var(--muted-foreground);
}

.verified-facts dd {
  min-width: 0;
  margin: 0;
  overflow-wrap: anywhere;
}

.verified-fact-detail,
.verified-fact-boundary {
  display: block;
  margin-top: 0.2rem;
  color: var(--muted-foreground);
  font-size: 12px;
}
```

Under the existing mobile media query, collapse `.verified-facts > div` to one column. Do not hide the definition list in print CSS.

- [ ] **Step 5: Run renderer tests and full Skill tests**

Run:

```bash
python3 -m unittest discover \
  -s skills/creating-model-doctor-reports/tests \
  -p 'test_*.py' -v
```

Expected: all tests pass; the final conclusion remains the second top-level section.

- [ ] **Step 6: Commit Task 3**

```bash
git add skills/creating-model-doctor-reports/scripts/model_doctor_html.py \
  skills/creating-model-doctor-reports/assets/report.css \
  skills/creating-model-doctor-reports/tests/test_model_doctor_v6.py
git commit -m "feat: render verified facts in final conclusions"
```

### Task 4: Teach the Skill to Author Evidence-Bounded Facts

**Files:**
- Modify: `skills/creating-model-doctor-reports/SKILL.md`
- Modify: `skills/creating-model-doctor-reports/references/evaluation-rules.md`
- Test: `skills/creating-model-doctor-reports/tests/test_model_doctor_v6.py`

- [ ] **Step 1: Add failing Skill-contract tests**

```python
def test_skill_requires_verified_protocol_context_and_concurrency_facts(self) -> None:
    skill = (SKILL_DIR / "SKILL.md").read_text(encoding="utf-8")
    for required in (
        "llm-capability-doctor.reviews.v2",
        "llm-capability-doctor.assessment.v6",
        "interfaceProtocol",
        "contextWindow",
        "highestVerifiedInputTokens",
        "firstFailedInputTokens",
        "highestVerifiedConcurrentRequests",
    ):
        self.assertIn(required, skill)

def test_rules_forbid_claiming_tested_values_as_hard_limits(self) -> None:
    rules = (SKILL_DIR / "references" / "evaluation-rules.md").read_text(encoding="utf-8")
    self.assertIn("最高已验证", rules)
    self.assertIn("不能写成真实硬上限", rules)
    self.assertIn("002", rules)
    self.assertIn("014-018", rules)
    self.assertIn("057", rules)
```

- [ ] **Step 2: Run tests and confirm RED**

Run:

```bash
PYTHONPATH=skills/creating-model-doctor-reports/tests python3 -m unittest \
  test_model_doctor_v6.ModelDoctorV6Tests.test_skill_requires_verified_protocol_context_and_concurrency_facts \
  test_model_doctor_v6.ModelDoctorV6Tests.test_rules_forbid_claiming_tested_values_as_hard_limits -v
```

Expected: FAIL because the Skill still documents reviews.v1/assessment.v5 and no verified-facts stage.

- [ ] **Step 3: Update `SKILL.md` workflow and envelope**

Make these contract changes:

```markdown
The output contract is `llm-capability-doctor.assessment.v6`.

After every per-test status is fixed, write `capabilitySummary.verifiedFacts` before grouping FAIL issues:

- `interfaceProtocol`: classify only from test 002 request/response structure as one of the five recognized families, `CUSTOM`, or `UNKNOWN`; describe both request and response shapes.
- `contextWindow`: use tests 014-018 to report the highest passed tier and first collected higher failure. Populate Token values only from provider-native usage fields. Never convert character counts to exact Tokens.
- `concurrency`: inspect every test 057 wave and report the highest wave whose complete sample set passed without rate limiting.
- Always say “最高已验证” or “至少支持”. Never call the highest tested value the real maximum or hard limit.
- When a corresponding manifest was not collected, use `NOT_COLLECTED` with null values and empty evidence references.
```

Replace the reviews example with `schemaVersion: llm-capability-doctor.reviews.v2` and include complete `verifiedFacts` examples for all three objects. Update final verification and reporting steps to require assessment.v6 and confirm all three rows render.

- [ ] **Step 4: Add the rules section**

Insert a new “Verified capability facts” section in `evaluation-rules.md` after cross-cutting rules. Include exact source ownership and boundaries:

```markdown
- Protocol format comes from 002 and names both request and response structure. A matched known protocol is not CUSTOM; an unmatched response is not automatically proven CUSTOM.
- Context comes from 014-018. Report the highest passing collected tier and first failing collected higher tier. Native input Token counts may be reported only when the response exposes them.
- Concurrency comes from all 057 requests. A wave counts only when every sample is semantically correct, has a valid metric, and is not rate-limited.
- “32 concurrent requests passed” means at least 32 short-run concurrency was verified; it does not mean the hard maximum is 32.
- Highest passing context and first failure describe the observed interval. They do not identify an exact hard limit or its root cause.
```

Renumber the contents and following sections consistently.

- [ ] **Step 5: Run Skill validation and tests**

Run:

```bash
python3 /Users/libolun/.codex/skills/.system/skill-creator/scripts/quick_validate.py \
  skills/creating-model-doctor-reports
python3 -m unittest discover \
  -s skills/creating-model-doctor-reports/tests \
  -p 'test_*.py' -v
```

Expected: `Skill is valid!` and all tests pass.

- [ ] **Step 6: Commit Task 4**

```bash
git add skills/creating-model-doctor-reports/SKILL.md \
  skills/creating-model-doctor-reports/references/evaluation-rules.md \
  skills/creating-model-doctor-reports/tests/test_model_doctor_v6.py
git commit -m "docs: require evidence-bounded capability facts"
```

### Task 5: Add a Complete v2 Fact Fixture and Run Release Verification

**Files:**
- Modify: `skills/creating-model-doctor-reports/tests/test_model_doctor_v6.py`

- [ ] **Step 1: Add a parsed fixture with protocol, context, and concurrency evidence**

Add a helper whose tests map contains 002, 016, 017, and 057. Use request IDs `protocol-openai`, `context-pass`, `context-fail`, `concurrency-32-1`, with metrics and response bodies that support:

```python
verified = {
    "interfaceProtocol": {
        "evidenceState": "VERIFIED",
        "family": "OPENAI_CHAT_COMPLETIONS",
        "requestFormat": "messages 数组与 role/content 消息。",
        "responseFormat": "choices[].message 与 chat.completion 结构。",
        "statement": "请求和响应采用 OpenAI Chat Completions 格式，不是 Anthropic 或自定义格式。",
        "evidenceRefs": ["request:protocol-openai"],
        "boundary": "协议结构不证明底层商业模型身份。",
    },
    "contextWindow": {
        "evidenceState": "VERIFIED",
        "highestVerifiedTier": "32K Token 近似档",
        "highestVerifiedInputTokens": 80175,
        "firstFailedTier": "64K Token 近似档",
        "firstFailedInputTokens": 131072,
        "statement": "最高已验证 80175 输入 Token，131072 输入 Token 的更高档首次失败。",
        "evidenceRefs": ["request:context-pass", "request:context-fail"],
        "boundary": "最高已验证值不是硬上限，真实上限未测试。",
    },
    "concurrency": {
        "evidenceState": "VERIFIED",
        "highestVerifiedConcurrentRequests": 32,
        "statement": "短时并发最高已验证到 32 个同时请求。",
        "evidenceRefs": ["request:concurrency-32-1"],
        "boundary": "32 是最高已验证波次，不代表服务硬上限或持续负载能力。",
    },
}
```

- [ ] **Step 2: Add an end-to-end assertion**

```python
def test_complete_verified_facts_survive_validate_assemble_and_render(self) -> None:
    parsed, reviews = self._complete_fact_fixture()
    self.assertEqual([], validate_reviews(parsed, reviews))
    assessment = assemble_assessment(parsed, reviews)
    self.assertEqual([], validate_assessment(assessment))
    html = render_report(assessment, ASSET_DIR)
    self.assertEqual(80175, assessment["capabilitySummary"]["verifiedFacts"]["contextWindow"]["highestVerifiedInputTokens"])
    self.assertEqual(32, assessment["capabilitySummary"]["verifiedFacts"]["concurrency"]["highestVerifiedConcurrentRequests"])
    self.assertIn("OpenAI Chat Completions", html)
    self.assertIn("真实上限未测试", html)
```

- [ ] **Step 3: Run the new test and full tracked suite**

Run:

```bash
PYTHONPATH=skills/creating-model-doctor-reports/tests python3 -m unittest \
  test_model_doctor_v6.ModelDoctorV6Tests.test_complete_verified_facts_survive_validate_assemble_and_render -v
python3 -m unittest discover \
  -s skills/creating-model-doctor-reports/tests \
  -p 'test_*.py' -v
```

Expected: focused test and full suite pass.

- [ ] **Step 4: Run full verification**

```bash
python3 /Users/libolun/.codex/skills/.system/skill-creator/scripts/quick_validate.py \
  skills/creating-model-doctor-reports
python3 -m compileall -q skills/creating-model-doctor-reports/scripts
git diff --check
git status --short
```

Expected: Skill valid, Python compilation succeeds, no whitespace errors, and only intended tracked changes remain before commit.

- [ ] **Step 5: Commit Task 5**

```bash
git add skills/creating-model-doctor-reports/tests/test_model_doctor_v6.py
git commit -m "test: cover verified report capability facts"
```

- [ ] **Step 6: Audit the final commit range**

```bash
git diff --stat f5c40f7..HEAD
git log --oneline f5c40f7..HEAD
git status --short
```

Expected: five focused implementation commits after the approved design commit and a clean worktree.
