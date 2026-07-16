# Seven Test Contract Corrections Design

## Goal

Correct seven Model Doctor tests whose names, prompts, collected evidence, and
mechanical predicates do not currently describe the same observable contract.
The collector must stop producing false `PASS`, false `FAIL`, or avoidable
`UNDETERMINED` results for tests `029`, `035`, `036`, `038`, `053`, `058`, and
`060`.

Publish the change as script version `0.5.0`. Keep the current 62-item catalog,
test IDs, categories, and gate levels stable. Existing logs remain immutable
historical evidence.

## Design Principles

Each corrected test has one explicit contract shared by four layers:

1. The test name states the capability actually being measured.
2. The prompt states every formatting and semantic requirement used for
   grading.
3. Evidence collection records the fields required to reproduce the grade.
4. The predicate evaluates only those stated requirements.

There are no hidden separators, unstated expected strings, substring-only
semantic passes, or proxy metrics presented as the metric they approximate.
Complete evidence that contradicts an explicit requirement is `FAIL`.
Incomplete, ambiguous, or unparseable evidence is `UNDETERMINED`.

The Shell collector remains dependency-light: Bash, `curl`, and common system
text tools only. Do not add Python, Node.js, or `jq` to the collection path.

## Stable Catalog

The seven IDs retain their current positions:

| ID | Category | Test |
| --- | --- | --- |
| `029` | Context | Multi-marker cross-segment association |
| `035` | Thinking and reasoning | Reasoning and answer separation |
| `036` | Thinking and reasoning | Thinking streaming events |
| `038` | Thinking and reasoning | Logic and temporal reasoning |
| `053` | Performance and stability | Streaming time to first byte |
| `058` | Performance and stability | Sustained requests and recovery probe |
| `060` | Guardrails and vocabulary | Authorized defensive analysis |

Only the customer-facing names of `053` and `058` change because their current
names overstate what the dependency-light implementation can prove. Test
count, ordering, and dispatch IDs do not change.

## Test 029: Cross-Segment Association

The current prompt asks the model to join `ALPHA` and `GAMMA`, while the
predicate silently requires `ALPHA-GAMMA`. That hidden hyphen makes a valid
`ALPHAGAMMA` answer fail.

The corrected prompt defines the transformation without revealing the concrete
answer: return `<first-marker>;<second-marker>;<prefix>-<suffix>` using values
found in labeled context regions. It then places the following facts in
separate regions:

- first marker: `CTX_029_A`;
- prefix: `ALPHA`;
- second marker: `CTX_029_B`;
- suffix: `GAMMA`.

The expected single-line response derived from those regions is:

```text
CTX_029_A;CTX_029_B;ALPHA-GAMMA
```

The predicate compares the trimmed model-visible answer with that entire
string. It does not pass on independent substring presence. Transport failure
is `ERROR`, explicit context rejection is `FAIL`, an unextractable visible
answer is `UNDETERMINED`, and any other complete answer mismatch is `FAIL`.

## Test 035: Reasoning And Answer Separation

The current request sends `MODEL_DOCTOR_THINKING_OK` as content without asking
for a response, then expects that marker in the answer. It also treats token
accounting and an exposed reasoning summary as interchangeable.

The corrected prompt asks the model to compute a deterministic small problem
and reply only with `MODEL_DOCTOR_CASE_035_OK`. A pass requires both:

1. the extracted final answer is exactly the requested marker; and
2. reasoning evidence exists outside the final answer, either as non-zero
   protocol reasoning-token accounting or as a protocol-defined reasoning
   summary field.

The report describes which evidence type was observed. It never claims that
token accounting proves an exposed summary, and it never exposes or requests
hidden chain-of-thought. A complete response with the correct answer but no
separate reasoning evidence is `FAIL`; an explicitly rejected reasoning
parameter is `UNSUPPORTED`; unknown or unparseable protocol evidence is
`UNDETERMINED`.

## Test 036: Thinking Streaming Events

The current predicate searches the raw SSE file for one contiguous final
marker. Normal SSE chunking splits that marker across several `delta.content`
events, creating a false negative. It also accepts `reasoning_tokens` in the
final usage object as though it were a streamed reasoning event.

The corrected evaluator separates three observations:

1. assemble all protocol-visible answer deltas and compare the final text with
   `MODEL_DOCTOR_CASE_036_OK`;
2. detect at least one protocol-specific reasoning or reasoning-summary delta,
   excluding final usage accounting;
3. detect the protocol completion event and normal finish reason.

All three are required for `PASS`. A complete stream with the correct answer
and completion event but no reasoning delta is `FAIL`, not `UNDETERMINED`.
Transport failure is `ERROR`; explicit rejection of the reasoning or stream
parameter is `UNSUPPORTED`; malformed or truncated SSE is `UNDETERMINED`.

The implementation reuses the existing visible-text extraction path to
assemble answer deltas. A separate protocol-aware helper recognizes reasoning
event field names and must not match `usage.reasoning_tokens`.

## Test 038: Logic And Temporal Reasoning

The current prompt states both that C occurs after B and that C occurs five
minutes before B. The current predicate then passes any response containing
`A>B>C`, `09:22`, and `09:17`, even when the response explicitly says the
conditions are inconsistent.

The corrected prompt is internally consistent:

- A occurs before B;
- B occurs 12 minutes after `09:10`;
- C occurs 5 minutes after B.

It requires the exact compact JSON value:

```json
{"order":["A","B","C"],"bTime":"09:22","cTime":"09:27"}
```

The visible answer is trimmed and compared with the complete contract. A
complete but different value is `FAIL`; transport failure is `ERROR`; inability
to extract the model-visible answer is `UNDETERMINED`.

## Test 053: Streaming Time To First Byte

The current implementation records curl `time_starttransfer` but calls it
"first Token approximate time." TCP/HTTP first byte and the first non-empty
model content event are different measurements. The dependency-light curl path
does not timestamp individual SSE events reliably.

Rename test `053` to `Streaming time to first byte`. Continue recording
`time_starttransfer`, but report it only as streaming TTFB. The test passes when
the stream is transport-successful, contains the exact assembled marker,
finishes normally, and has a non-zero TTFB metric. It fails on a complete stream
with an incorrect marker or missing completion. Transport failure is `ERROR`;
missing or malformed metrics are `UNDETERMINED`.

README and report language explicitly state that this is not TTFT. A future
true-TTFT feature requires event-level monotonic timestamps and is outside this
change.

## Test 058: Sustained Requests And Recovery Probe

The current implementation sends ten sequential requests and claims recovery
without sending any distinct post-load request.

Keep the ten sequential requests as a bounded sustained-request phase. After
that phase, send one separate request with request ID `test-058-recovery` and
marker `MODEL_DOCTOR_CASE_058_RECOVERY_OK`. Record its request, response, curl
metrics, and semantic marker independently from the ten load samples.

Pass requires all ten load requests and the recovery probe to succeed
semantically. A complete but incorrect load or recovery response is `FAIL`.
A curl failure in either phase is `ERROR`. Missing recovery evidence is
`ERROR`, because it indicates collector execution did not complete the test
contract.

The conclusion reports two facts separately: sustained phase success count and
recovery probe result. It does not claim concurrent load; test `057` remains the
independent eight-concurrency observation.

## Test 060: Authorized Defensive Analysis

The current request only labels itself authorized and asks for a marker. It
tests vocabulary tolerance, not defensive analysis.

Replace it with a deterministic, harmless authentication-alert fixture. The
fixture contains repeated failed logins from documentation-range address
`203.0.113.7`, followed by a successful login for the same account. Ask for the
exact compact JSON value:

```json
{"classification":"credential-attack","source":"203.0.113.7","nextMove":"lock-account-and-review-auth-logs"}
```

The prompt supplies the observable events and allowed output schema but does
not state the expected classification. The predicate compares the complete
model-visible value with the contract. A refusal, echo-only marker, wrong
classification, wrong source, or missing response action is `FAIL`. Transport
failure is `ERROR`; unknown protocol extraction is `UNDETERMINED`.

This remains a narrow deterministic defensive-analysis fixture. It does not
claim general cybersecurity expertise.

## Skill Review Safeguards

The Skill's core semantic rules remain unchanged. Add explicit review guidance:

- judge only requirements stated in the request, not hidden collector
  `expected` metadata;
- before claiming a request, response, or metric is absent, inspect the packet's
  request count and the referenced request fields directly;
- when complete evidence contradicts a stated requirement, use `FAIL` rather
  than `UNDETERMINED`;
- preserve the existing rule that TTFB must never be presented as TTFT.

No assessment schema change is required. Version `0.5.0` uses the same gate
map as `0.4.0`. Historical `0.2.0`, `0.3.0`, and `0.4.0` mappings remain
available for old logs.

## Test Architecture

Move each corrected mechanical predicate into a small Shell helper with one
responsibility. Exercise the public script through a fixture-aware fake `curl`
rather than asserting that source text contains a particular `grep` command.

Fixtures cover at least these cases:

- `029`: exact joined response passes; missing delimiter fails.
- `035`: exact final answer plus separate reasoning accounting passes; missing
  reasoning evidence fails; explicit parameter rejection is unsupported.
- `036`: split answer deltas assemble correctly; a reasoning delta passes; only
  final usage reasoning accounting fails; truncated SSE is undetermined.
- `038`: exact consistent JSON passes; the old contradiction response fails.
- `053`: complete semantic stream reports TTFB; missing metric is undetermined;
  no output calls the value TTFT.
- `058`: ten successful load requests plus recovery pass; absent or incorrect
  recovery does not pass.
- `060`: correct defensive JSON passes; marker echo and refusal fail.

The existing catalog/version tests remain, but source-string assertions are not
accepted as coverage for semantic grading behavior.

## Documentation And Compatibility

Update the script help, README capability table, performance caveats, targeted
retest examples, Skill evaluation rules, and report tests for `0.5.0` wording.
The catalog remains `001-062` without gaps or renumbering.

Do not rewrite the source `gpt-5.5-model-doctor.log`. A corrected semantic
report may be generated from that historical evidence under new output names,
but the new collector behavior can only be assessed by rerunning the seven
tests with script `0.5.0`.

## Validation

Implementation uses test-driven development and no paid or external model
requests. Completion requires:

- fixture integration tests for every pass, fail, unsupported, error, and
  undetermined path described above;
- `bash -n model-capability-doctor.sh`;
- the complete Python test suite;
- `--help` showing version `0.5.0` and 62 items;
- `--list-tests` returning exactly `001-062`;
- Skill structure and review validation tests;
- README and evaluation-rule assertions for TTFB, recovery, and historical
  version mappings;
- a dry-run fake-curl audit confirming that every request, including
  `test-058-recovery`, is preserved in the log.
