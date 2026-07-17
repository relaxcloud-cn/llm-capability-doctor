# Adaptive Concurrency Stress Test Design

## Summary

Replace the current short concurrency snapshot in test `057` with an opt-in,
auditable stress test that determines the client-observed best concurrency for
a standardized model workload. The result must identify the concurrency level
that maximizes successful output-token throughput while preserving semantic
success and an adaptive P95 latency guardrail.

The standard run uses one workload profile, tests up to concurrency `64`, and
targets an 8-12 minute runtime. It first evaluates a coarse power-of-two ladder,
then tests one or two intermediate levels around the best coarse result.

## Goals

- Measure sustained behavior at each concurrency level instead of launching one
  finite request wave.
- Report successful requests per second, output tokens per second, complete
  response latency, semantic success, throttling, and transport failures.
- Recommend one best concurrency and a stable concurrency interval.
- Keep the complete run reproducible and auditable from the Model Doctor log.
- Protect customer endpoints with an explicit maximum concurrency and severe
  overload stop conditions.
- Preserve the current single-script, curl-based customer-site workflow.

## Non-Goals

- Proving a provider SLA or the absolute capacity of the upstream model fleet.
- Measuring server-side accelerator utilization or queue depth.
- Isolating internet, gateway, and model execution time from one another.
- Measuring TTFT or TPOT in the first version. Curl `time_starttransfer` remains
  TTFB and must not be relabeled as TTFT.
- Testing several prompt-length or output-length profiles in one run.
- Automatically raising the maximum concurrency above `64`.

## Current Behavior and Gap

Test `057` currently launches synchronized waves at concurrency `4`, `8`, `16`,
and `32`. Each worker performs one short exact-marker request. The test reports
semantic success count, rate-limit count, P50, nearest-rank P95, and maximum
complete-response latency.

That design is a useful short capacity snapshot, but it cannot determine the
best concurrency because it does not measure steady-state throughput, uses an
unrepresentatively small response, has no baseline-relative latency guardrail,
and does not refine the search around a promising concurrency level.

## Selected Approach

Use a two-stage adaptive search.

1. Run a coarse ladder at concurrency `1`, `2`, `4`, `8`, `16`, `32`, and `64`.
2. Determine the best eligible coarse level using successful output-token
   throughput.
3. Test one intermediate level between the best coarse level and each existing
   neighbor when that midpoint is a distinct integer.
4. Recompute the recommendation from all measured levels.

For example, when concurrency `16` is the best coarse result, the refinement
stage tests `12` and `24`. If an edge level wins, only its available neighbor is
refined. Every reported recommendation is therefore a measured value, never an
interpolated value.

This approach is preferred over a fixed ladder because it gives a more useful
recommendation, and preferred over continuous hill climbing because throughput
and latency are noisy and not guaranteed to be monotonic.

When a configured maximum is lower than `64`, the coarse ladder contains every
power-of-two level not exceeding that maximum and also the exact maximum when it
is not already present. The approved standard configuration uses maximum `64`.

## Invocation and Safety

The sustained stress test must be explicitly enabled because it is materially
more expensive and more disruptive than the current catalog run. The intended
interface is:

```bash
./model-capability-doctor.sh \
  --url 'https://model.example/v1/messages' \
  --model 'customer-model' \
  --api-key '...' \
  --only '057' \
  --stress-mode standard \
  --stress-max-concurrency 64 \
  --log-file './stress-test.log'
```

Supported stress modes:

- `off`: do not run the sustained stress test. Test `057` is `SKIPPED` with a
  concrete rerun command.
- `standard`: run the best-concurrency test defined by this specification.

The command-line default remains `off` so a normal all-catalog run cannot
silently generate sustained load. `--stress-max-concurrency` defaults to `64`
when a stress mode is enabled and rejects values below `1` or above `64`.

The script prints the selected mode, maximum concurrency, workload, expected
runtime range, and the fact that the run generates billable model traffic before
starting test `057`. It remains non-interactive so it is usable in automation.

## Standard Workload

Each measured request uses:

- a deterministic synthetic, non-sensitive payload containing approximately
  4,096 ASCII characters, targeting roughly 1,000 input tokens while recording
  the actual provider-reported count;
- a request for approximately 256 visible English words, accepted only when it
  contains 200-320 whitespace-delimited words before the final marker;
- a unique request nonce near the beginning of the input to prevent accidental
  prefix-cache reuse from dominating the result;
- a required final marker tied to that nonce; and
- a response budget large enough to produce the requested text and marker.

The marker verifies that the visible response completed the assigned request.
HTTP `2xx` alone is not a successful sample. The response is successful only
when transport, HTTP, protocol completion, visible marker, and usable timing
evidence all pass.

The workload generator must be deterministic apart from the nonce. The report
states that this synthetic profile represents one standardized workload and
does not predict every application workload.

The script has no model-specific tokenizer and therefore records the generated
payload size and provider-reported input tokens as separate facts. Provider
`output_tokens` can include reasoning or other non-visible tokens; the report
labels tokens/s as provider-reported output-token throughput, not visible-token
generation speed.

## Sampling Schedule

### Baseline

Concurrency `1` establishes the latency baseline. It performs two unmeasured
warm-up completions, then measures for at least 45 seconds. Measurement may
extend to 90 seconds to obtain 15 completed samples. If fewer than 10 successful
samples are available at the hard limit, the recommendation is
`UNDETERMINED` because the adaptive P95 guardrail is not credible. A baseline
with 10-14 successful samples is allowed but lowers recommendation confidence
and is called out in the report.

### Coarse Levels

Each remaining coarse level has:

- a 5-second warm-up period excluded from statistics;
- a 30-second measurement window;
- closed-loop workers, where each worker starts its next request immediately
  after its previous request completes; and
- a drain phase that waits for requests started before the measurement cutoff.

After warm-up requests have fully drained, the measurement clock starts and all
workers are released together. Every request started before the 30-second cutoff
is a measured sample, including a request that completes during drain. No new
request starts after the cutoff. The throughput denominator is the elapsed time
from the synchronized measurement start until the final measured request
completes, so slow in-flight work is penalized rather than omitted.

The run proceeds in ascending order to limit load surprises.

### Refinement Levels

Each distinct midpoint around the best eligible coarse level uses the same
5-second warm-up and 30-second measurement window. Refinement levels are run in
ascending order.

The standard run is expected to finish in 8-12 minutes, including baseline
extension, request drain time, refinement, and endpoint latency variation.

## Per-Sample Data

Every sample receives a stable request ID and records:

- concurrency level and worker index;
- warm-up or measured classification;
- monotonic offset from the level start where available;
- curl exit code and HTTP status;
- complete-response latency from curl `time_total`;
- TTFB from curl `time_starttransfer`, labeled only as TTFB;
- semantic-success boolean and failure reason;
- rate-limit, timeout, transport-error, and HTTP 5xx classification;
- reported input, cache-read, cache-creation, and output tokens when available;
- redacted request and response evidence; and
- request start and completion timestamps.

Warm-up samples remain auditable but do not contribute to measured aggregates.
Measured samples that finish during drain remain in all measured counts,
latency distributions, success rates, and the throughput denominator.

## Per-Level Metrics

For measured samples at each concurrency level, calculate:

- attempted, completed, and semantically successful request counts;
- semantic success rate;
- successful requests per second;
- aggregate input and output tokens;
- successful output tokens per second;
- P50, nearest-rank P95, nearest-rank P99, and maximum complete-response latency;
- HTTP 429 count and rate;
- HTTP 5xx count and rate;
- timeout and other transport-error counts and rates; and
- wall-clock warm-up, measurement, and drain durations.

Output-token throughput is available only when every semantically successful
measured response has a valid output-token count. If any successful sample lacks
that count, the level's token throughput is `not_available`; successful RPS is
still reported.

Percentiles are calculated only from semantically successful measured samples.
The report also shows failed-sample counts so excluding them cannot make an
unhealthy level appear healthy.

P99 from fewer than 100 successful samples is a short-run tail observation and
can equal the maximum. The customer report states the sample count next to every
percentile and does not describe these values as SLA estimates.

## Eligibility and Recommendation

Let `baseline_p95` be the P95 complete-response latency at concurrency `1`.
The adaptive latency limit is:

```text
latency_limit = 2.0 * baseline_p95
```

A concurrency level is eligible only when:

- it has at least one measured completion;
- its semantic success rate is at least 99%;
- when it has fewer than 100 measured attempts, it has zero failed attempts;
- its P95 complete-response latency is at most `latency_limit`;
- it has no missing latency values among semantic successes; and
- it did not trigger a severe overload stop.

The primary score is successful output tokens per second. When token throughput
is unavailable for every otherwise eligible level, the run falls back to
successful requests per second and states that fallback prominently. Mixed
token-throughput availability does not permit comparing token-scored and
RPS-scored levels in one recommendation; it produces `UNDETERMINED` unless all
eligible candidates share the same score type.

Find the highest primary score among eligible levels. Any eligible level within
3% of that score belongs to the performance plateau. Recommend the lowest
concurrency in the plateau to avoid spending concurrency for noise-level gains.

The stable interval is the maximal contiguous sequence, in ascending order of
measured concurrency, that contains the recommendation and whose levels are
eligible with primary scores at least 97% of the maximum. If only the
recommended level qualifies, the stable interval is that single measured level.

## Severe Overload Stops

Do not start a higher concurrency level when the latest completed level meets
any of these conditions:

- semantic success rate is below 90%;
- combined 429, 5xx, timeout, and transport-error rate is at least 10%;
- P95 complete-response latency exceeds four times `baseline_p95`; or
- three consecutively completed samples at the level are request timeouts before
  the measurement window ends.

The current level is retained in the evidence and marked ineligible. Untested
higher levels are reported as stopped for endpoint protection, not as failed or
unsupported. Refinement never tests a value above the severe-stop level.

## Result Status

Test `057` uses the following reviewed outcome semantics:

- `PASS`: a recommendation is available, all evidence required for its score is
  complete, and at least one level above concurrency `1` is eligible.
- `FAIL`: measured evidence confirms overload or contract failure at all tested
  levels above concurrency `1`.
- `UNDETERMINED`: baseline evidence, score comparability, sample completion, or
  recommendation evidence is insufficient or ambiguous.
- `SKIPPED`: stress mode is `off`.
- `ERROR`: the runner or transport fails in a way that prevents a meaningful
  measured ladder.

An overload at a higher level does not by itself fail the test when a lower
eligible recommendation is available. Finding the boundary is expected behavior
for a stress test.

## Log Contract

The raw log adds a structured stress summary after the request audits. It
contains versioned, parseable key-value records for:

- stress configuration and workload profile;
- baseline and adaptive latency limit;
- every measured level and all aggregate metrics;
- score type and any fallback reason;
- coarse and refinement level identities;
- severe-stop reason, when present;
- recommended concurrency and stable interval; and
- explicit limitations.

Every request remains linked to test `057`. Request bodies, response bodies, and
credentials continue through the existing redaction and audit path. The source
log remains the evidence of record; the report generator must not reconstruct
missing metrics from conclusions.

## Customer Report

The category summary must no longer say only `8/8 通过`. It must surface:

- recommended concurrency;
- stable interval;
- peak successful output tokens per second or RPS fallback;
- success rate and P95 at the recommendation;
- adaptive P95 limit;
- maximum tested concurrency and any safety stop; and
- the standardized-workload and non-SLA limitation.

Test `057` renders a compact level table with columns for concurrency, measured
requests, success rate, successful RPS, output tokens/s, P50, P95, P99, 429,
errors, and eligibility. The recommendation row is visibly identified without
hiding other measured levels.

Customer wording must say that the recommendation is the best measured
client-observed concurrency for this endpoint, region, network path, workload,
and time window. It must not claim upstream fleet capacity or a durable SLA.

## Components and Boundaries

The implementation should keep the following responsibilities separate:

1. Workload builder: creates the standardized request and nonce-specific marker.
2. Stress scheduler: manages workers, warm-up, fixed measurement windows, drain,
   ascending levels, refinement, and safety stops.
3. Sample validator: classifies protocol completion and semantic success and
   extracts usage without making aggregate decisions.
4. Metrics reducer: converts sample records into level distributions and rates.
5. Recommendation engine: applies eligibility, plateau, refinement, and fallback
   rules to complete level metrics.
6. Log renderer: writes auditable request records and the versioned stress
   summary.
7. Report parser and renderer: validates the summary and presents the final
   recommendation without reinterpreting raw performance data incorrectly.

The customer-facing entry point remains the Bash script. Small embedded or
companion logic is acceptable only when it does not add a customer-site runtime
dependency beyond tools already required by the repository.

## Error Handling

- A failed sample is recorded and classified; it never disappears from the
  denominator.
- A worker failure must not terminate other workers or corrupt the level summary.
- Missing or malformed curl metrics make that sample unsuccessful.
- Missing token usage disables token throughput for that level but does not
  discard valid latency and RPS evidence.
- Interrupted runs retain completed request audits and clearly mark the active
  level incomplete.
- Temporary worker files remain inside the secure run directory and use the
  existing cleanup path.
- Unknown protocols cannot run the standardized semantic validator and produce
  `UNDETERMINED`, not a successful performance recommendation.

## Testing Strategy

Focused automated tests must use a deterministic fake endpoint or fake curl and
short configurable windows; the test suite must never wait 8-12 minutes.

Coverage must verify:

- stress mode defaults to `off` and `057` reports `SKIPPED` with rerun guidance;
- option validation rejects concurrency outside `1-64`;
- closed-loop workers continue issuing requests during a measurement window;
- warm-up samples are audited but excluded from measured aggregates, while
  measured requests completed during drain remain included;
- semantic failures and malformed metrics remain in the attempt denominator;
- output-token throughput and RPS use the measured wall-clock window;
- P50, nearest-rank P95, nearest-rank P99, and maximum are correct;
- eligibility enforces zero failures below 100 attempts and 99% thereafter;
- the adaptive limit is exactly twice the measured concurrency-1 P95;
- severe-stop conditions prevent higher levels from starting;
- refinement chooses the correct midpoints and never invents an unmeasured
  recommendation;
- the 3% plateau chooses the lower concurrency;
- token-metric fallback and mixed-availability ambiguity are handled exactly;
- every sample is linked to test `057` and credentials remain redacted;
- the parser rejects inconsistent aggregate counts or recommendation fields;
- the HTML summary and level table show the same recommendation and metrics as
  the assessment artifact; and
- shell syntax checks and the complete report parser/renderer tests pass.

## Acceptance Criteria

The feature is complete when a standard `--only 057 --stress-mode standard` run:

- executes the standardized workload through the coarse and applicable
  refinement levels without exceeding concurrency `64`;
- runs sustained closed-loop measurement windows rather than one finite wave;
- produces auditable per-sample and per-level evidence;
- applies endpoint-protection stops exactly as specified;
- reports successful RPS, output tokens/s when available, semantic success,
  latency distributions, throttling, and errors;
- recommends a measured concurrency using the eligibility and plateau rules;
- reports a stable measured interval and the adaptive P95 limit;
- renders the result prominently in the customer report; and
- states the workload, environment, sample, and SLA limitations without
  overstating the conclusion.
