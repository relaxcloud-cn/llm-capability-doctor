# 4-32 Concurrent Response-Latency Ladder Design

## Goal

Replace the single-wave `057 / 8 并发性能` check with one auditable concurrency
ladder that measures complete-response latency at 4, 8, 16, and 32 concurrent
requests.

The check reports observed latency without inventing an SLA. Pass or failure is
determined by request correctness, not by a fixed latency threshold.

## Scope

This change:

- retains test ID `057` and the 62-item catalog;
- renames test `057` to `4-32 并发响应时间`;
- executes four sequential concurrency waves at `4`, `8`, `16`, and `32`;
- reports per-wave semantic success, rate limits, P50, nearest-rank P95, and
  maximum complete-response latency;
- preserves per-request evidence in the audit log;
- makes the generated assessment and HTML report show the ladder summary instead
  of the last request's metrics; and
- increments the script contract version from `0.5.0` to `0.6.0` while retaining
  historical report mappings.

This change does not add sustained load, throughput, TTFT, token-generation
speed, or an externally defined latency SLA. Test `058` remains responsible for
the existing sustained-load and recovery observation.

## Chosen Approach

The shell script executes and summarizes all four waves. This keeps measurement,
pass/fail evaluation, and auditable evidence together in the existing
single-file detector. It avoids introducing a load-testing dependency and does
not defer primary statistics to the report generator.

Alternative approaches were rejected:

- deriving statistics only in the Python report layer would leave the detector's
  own result incomplete and split one contract across two components;
- introducing `k6`, `vegeta`, or another load-testing tool would improve advanced
  load generation but would violate the detector's low-dependency deployment
  model for this bounded requirement.

## Execution Model

Test `057` runs the following waves in order:

1. 4 concurrent requests
2. 8 concurrent requests
3. 16 concurrent requests
4. 32 concurrent requests

Each wave launches all workers against a shared start barrier so that process
setup time does not intentionally serialize request start. The next wave begins
only after all workers in the current wave finish. A full successful run sends
60 test requests, excluding normal protocol detection requests.

Every wave uses a short, deterministic prompt with a wave-specific exact marker.
A request is semantically successful only when all of these conditions hold:

- curl exits with code `0`;
- the HTTP status is in the `2xx` range; and
- the response's visible model text equals the expected marker.

An HTTP success without the exact semantic marker is a failed sample. This
prevents an error envelope or unrelated model output from being counted as
concurrency capacity.

## Measurements

`curl`'s `time_total` is the complete-response latency for one request. It is
converted to integer milliseconds before aggregation. It is not TTFT, TTFB, or
token-generation speed and must not be described as any of those metrics.

For each wave, latency statistics are computed only from semantically successful
samples:

- P50: nearest-rank percentile at `ceil(0.50 * n)`;
- P95: nearest-rank percentile at `ceil(0.95 * n)`;
- maximum: the largest successful complete-response latency; and
- sample count: the number of semantic successes over the configured concurrency.

The wave also reports the count of HTTP `429` responses. Failed and rate-limited
request latencies remain available in raw evidence but do not distort the
successful-response distribution. If a wave has no semantically successful
samples, its P50, P95, and maximum values are `not_available`.

These values are a short-run snapshot. In particular, P95 for four samples is
the maximum sample and must not be represented as an SLA.

## Status Rules

Latency values have no pass/fail threshold.

- `PASS`: all 60 requests are semantically successful.
- `FAIL`: any wave contains a curl failure, non-2xx response, rate limit, or
  semantic mismatch.
- `UNDETERMINED`: a prerequisite prevents the script from constructing or
  executing a meaningful concurrency request, such as an unknown protocol.

A failed wave does not stop the test. The script continues through all remaining
waves so that the result contains the most complete observable ladder possible.

Test `057` remains an `important` readiness gate. Therefore, a failed or
undetermined result continues to make the overall assessment conditional rather
than becoming a critical blocker.

## Log Contract

The test-level `detected` field contains a stable, machine-readable summary in
ascending concurrency order. Its logical shape is:

```text
c4:success=4/4,p50_ms=1200,p95_ms=1800,max_ms=1800,rate_limited=0;c8:success=8/8,p50_ms=...
```

All four segments are present, including failed waves. Unavailable statistics
use the literal `not_available`.

The Chinese `conclusion` gives a compact customer-readable summary of all four
waves and states whether all requests were semantically successful. It does not
claim a latency SLA.

Raw evidence contains one section per request with at least:

- concurrency level;
- request index;
- HTTP status;
- curl exit code;
- semantic-success flag;
- complete-response latency in milliseconds; and
- response body.

The existing request audit blocks remain the authoritative per-request evidence
and retain headers, timing metrics, timestamps, redaction, and request/response
linkage. Request IDs include the wave label (`c4`, `c8`, `c16`, or `c32`) so the
report parser can associate all 60 requests with test `057`.

## Report Behavior

For test `057`, assessment assembly uses the test-level `detected` ladder as the
raw observation shown in HTML. It must not display only the last request's HTTP
status and latency, because that would misrepresent the multi-request check.

The report's semantic review should explain:

- that the four waves contain 4, 8, 16, and 32 samples respectively;
- which waves, if any, contain failures or rate limits;
- the P50, nearest-rank P95, and maximum complete-response latency per wave; and
- that the observation is a short-run snapshot rather than an SLA or sustained
  throughput result.

Historical `0.3.0`, `0.4.0`, and `0.5.0` catalog gate mappings remain unchanged.
Version `0.6.0` receives a new mapping with the same 62 IDs and gate levels
because test `057` keeps its identity and readiness importance while changing
its contract.

## Error Handling

Each parallel worker always writes a metrics file, response body, stderr, exit
code, start timestamp, and completion timestamp where possible. Missing or
malformed metrics are treated as failed samples with unavailable latency rather
than silently becoming zero-duration successes.

The parent waits for every worker and records every request even when individual
workers fail. Temporary barrier and sample files live under the existing secure
run directory and are removed by the current cleanup path.

## Testing

Automated coverage will verify:

- the catalog retains IDs `001` through `062` and renames `057`;
- test `057` launches exactly `4 + 8 + 16 + 32 = 60` requests with correctly
  labeled request IDs;
- HTTP `2xx` responses with the wrong marker are not counted as successes;
- P50, nearest-rank P95, and maximum values are correct for every wave;
- a failed early wave does not prevent later waves from running;
- HTTP `429` responses are counted and make the overall test fail;
- a wave with zero successful samples emits `not_available` statistics;
- parsed evidence links all concurrency requests to test `057`;
- assessment and HTML output display the four-wave summary instead of the final
  request alone;
- version `0.6.0` has a complete gate mapping while historical mappings remain
  available; and
- shell syntax checks and the complete Python test suite pass.

## Acceptance Criteria

The feature is complete when a `--only 057` run produces one test result that:

- performs the four specified concurrency waves;
- records 60 auditable test requests;
- reports per-wave success count, rate-limit count, P50, nearest-rank P95, and
  maximum complete-response latency;
- bases success on exact model output rather than HTTP status alone;
- applies no latency threshold;
- continues after partial failure;
- renders the full ladder summary in the customer report; and
- clearly describes the measurements as complete-response latency from a
  short-run snapshot.
