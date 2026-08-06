# Hide Freeform Final Summary Design

## Goal

Remove the freeform headline, grouped issue list, and scope boundary from the customer-facing HTML final-conclusion section. Keep the deterministic general verdict, four-value fact strip, and three verified-fact rows.

## Data Boundary

Keep `capabilitySummary.headline`, `issues`, and `scopeBoundary` in reviews and assessment JSON. They remain validated audit data and continue to cover every FAIL. This change affects presentation only and does not migrate the v6 assessment schema.

## Rendering

`_capability_summary` renders, in order: the heading, program-generated general verdict, and verified facts. It must not render the freeform summary fields. Remove CSS selectors that become unreachable.

## Verification

Add a regression test that supplies unique headline, issue, boundary, reference, and scope text and proves none appears in HTML while the fixed verdict and verified facts remain. Update existing renderer and skill-contract tests, run the full skill suite, validate the skill, regenerate the existing Doubao v6 report, and inspect desktop and mobile output.
