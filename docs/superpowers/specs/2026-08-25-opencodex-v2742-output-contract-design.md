# OpenCodex v2.7.42 Output Contract Design

**Status:** Approved for implementation

## Goal

Let the offline Rust CLI decide whether a model endpoint's actual responses can be read and relayed by OpenCodex v2.7.42 for these three adapters:

- `openai-chat`
- `anthropic`
- `google`

The customer runs one Rust executable. They do not install OpenCodex, Bun, Node.js, Docker, or any package manager, and the executable does not download rules at runtime.

## Fixed Upstream Contract

Every rule is derived from exactly this OpenCodex release:

```text
tag: v2.7.42
commit: 34493b12666a5fd69d69d730b13eabb5ec9d7235
license: MIT
```

The development-time rule generator reads the tagged adapter implementations, bridge implementation, and official tests. The generated contract is checked into this repository and embedded into the Rust executable. A future OpenCodex release requires a new contract version and a new CLI release; it cannot silently change a customer's result.

## Scope

The compatibility verdict covers the model-output side of the three selected adapters. It answers whether the endpoint accepts the adapter's request shape and returns data that OpenCodex v2.7.42 can parse, turn into its internal events, and relay to Codex.

The existing connection, API-key, timeout, long-context, performance, and general capability checks remain separate. They do not change the OpenCodex output verdict.

Out of scope for this contract are adapters with requirements that cannot be inferred from a generic model response alone: `azure`, `azure-openai`, `mimo-free`, `kiro`, and `cursor`.

## Discovery And Verdicts

The CLI does not accept an adapter-selection flag. It evaluates all three candidates independently:

```text
OpenAI Chat request      -> evaluate OpenAI Chat output contract
Anthropic Messages request -> evaluate Anthropic output contract
Google GenerateContent request -> evaluate Google output contract
```

Each candidate has one final status:

- `PASS`: every required rule for that adapter passed.
- `FAIL`: one or more required rules failed. An HTTP rejection, a response in another protocol shape, a malformed stream, or a missing required final event is a failure for that adapter.

The CLI must not choose the first candidate that works. A model can pass more than one adapter, and each adapter receives its own conclusion.

## Complete Output-Side Rule Families

The release artifact contains one rule for every applicable output-side branch in the pinned OpenCodex adapter and bridge tests. The list below is the required coverage boundary; it is not a five-test shorthand.

### Shared Stream And Completion Rules

- HTTP success versus an upstream error envelope.
- Valid framing for the adapter's response stream, including chunk boundaries, multiline records where that adapter accepts them, malformed JSON, and truncated final records.
- Correct root event or response shape before any content is accepted.
- Allowed keepalive and extension records without letting them disguise malformed model data.
- A valid native terminal signal exactly once.
- No content, tool call, malformed frame, or conflicting terminal after the response has completed.
- Correct distinction between success, model-incomplete, protocol-invalid, and explicit upstream error.

### Text, Thinking, And Usage Rules

- The selected response/candidate/message object is the expected one for the adapter.
- Text is extracted only from the adapter's valid text field or content block.
- Empty candidate/content structures are rejected when the response is supposed to contain text.
- Thinking or reasoning blocks are accepted only in their adapter-native location and type.
- Usage is read only from valid usage fields and cannot appear on a data frame where OpenCodex rejects it.
- Optional extension fields remain forward compatible without replacing native fields.

### Tool Call Rules

- Tool-call container, item, call ID, tool name, index, and argument fields have their adapter-native types.
- Tool names required for dispatch are nonempty; a tool name is never invented by the parser.
- Arguments may arrive in valid fragments or all at once, but their final accumulated value must be valid for the call.
- Parallel calls preserve their native order and do not merge unrelated indexes or IDs.
- Duplicate, conflicting, blank, sparse, or uncorrelated calls become non-executable failures.
- A finished stream cannot retain a partial or dangling tool call.

### Tool Result And Follow-Up Rules

- Every supplied tool result is mapped to exactly one model-emitted call using the adapter's native correlation field.
- The follow-up request preserves the assistant call and result history in the adapter's required order.
- Missing, duplicate, mixed, or mismatched result IDs fail before a follow-up is sent.
- After one tool result and after consecutive tool results, the model must return a valid next turn and then a valid final completion.

### Codex Relay Rules

- Parsed text becomes the expected Responses text event sequence.
- Parsed reasoning becomes the expected Responses reasoning sequence when present.
- Parsed tool calls become the expected output-item, argument-delta, and argument-complete events.
- Tool and custom-tool names/namespaces survive the relay without collision.
- A response has one valid terminal event: completed, failed, or incomplete; it cannot emit two successful terminals.
- The completed response includes the fields that OpenCodex v2.7.42 requires for strict Codex clients, including normalized usage details.

## Rule Artifact And Offline Audit

The embedded contract is a structured artifact, not a prose document and not a collection of clickable links. Every rule contains:

```text
id: OCX-CHAT-TOOL-004
adapter: openai-chat
requirement: Streaming tool-call function.name must be a nonempty string.
source_version: v2.7.42
source_commit: 34493b12666a5fd69d69d730b13eabb5ec9d7235
source_file: src/adapters/openai-chat.ts
source_file_sha256: generated 64-character SHA-256 for the exact v2.7.42 source file
source_test: exact v2.7.42 official test-file path for this rule
severity: required
```

The artifact has its own SHA-256 digest. The evidence log and report record that digest, so a customer can establish which offline contract produced the result. Runtime output includes source file paths and digests, not external URLs.

## Report Format

The report contains a direct pass/fail verdict for each of the three adapters. A failure identifies the exact rule, the OpenCodex requirement, the actual observed response location, and the resulting effect.

```text
OpenCodex v2.7.42 / openai-chat: FAIL

OCX-CHAT-TOOL-004: FAIL
Required by OpenCodex: streaming tool-call function.name is a nonempty string.
Observed response: choices[0].delta.tool_calls[0].function.name was an object.
Effect: OpenCodex cannot create a Codex tool-call event from this model response.
```

The report does not use `unsupported` or `analysis unavailable` for these three contracts. An endpoint that rejects a candidate request or returns a different shape fails the corresponding adapter with the observed HTTP status or response path.

Secrets, signed URLs, request contents, and response values remain redacted. Structural paths, type names, event names, and rule IDs remain visible because they are necessary to remediate the incompatibility.

## Implementation Boundaries

- The Rust code makes compatibility decisions deterministically. The `--self-analyze` model review may describe evidence but cannot change an OpenCodex pass/fail verdict.
- Tests are written first for each rule family and must include passing and failing fixtures derived from the pinned official tests.
- The production implementation contains only the three selected adapter contracts and the shared relay contract. It does not embed or execute OpenCodex TypeScript.
- The build/release workflow verifies the upstream tag, pinned commit, and source-file digests before regenerating the contract artifact.

## Acceptance Criteria

1. Running the CLI against fixture endpoints produces independent `PASS` or `FAIL` results for OpenAI Chat, Anthropic, and Google.
2. Every failure names a stable `OCX-*` rule, an observed response path, the expected OpenCodex v2.7.42 requirement, and a plain-language effect.
3. A valid stream, malformed stream, missing terminal, invalid tool name, invalid tool ID, fragmented arguments, parallel calls, mismatched tool result, and duplicate terminal each have regression coverage where applicable to the adapter.
4. The CLI runs with no OpenCodex, Bun, Node.js, Docker, download, or live source lookup in the customer environment.
5. The embedded artifact, evidence log, and report identify OpenCodex v2.7.42, the pinned commit, and the contract digest.
