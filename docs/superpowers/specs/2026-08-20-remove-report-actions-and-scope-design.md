# Remove Report Actions And Scope Section

## Goal

Remove the two UI areas marked in the supplied report screenshot from all newly rendered Model Doctor HTML reports and from the current `deepseek-v4-flash` report:

1. The sidebar action group containing `展开全部`, `收起全部`, and `打印报告`.
2. The complete `结果适用范围` report section, including its desktop and mobile navigation entries.

## Design

The report renderer remains the single source of truth. `model_doctor_html.py` will stop rendering the sidebar action group and scope section, and its navigation catalog will contain only the four remaining report sections. The now-unused scope constants and renderer helper will be removed.

`report.js` will retain search, filtering, mobile navigation, and active-section tracking, while removing only the event handlers for the three deleted buttons. `report.css` will remove only selectors used exclusively by the deleted action group and scope list.

The repository report skill instructions will describe and verify the four-section reading path instead of requiring `结果适用范围`.

## Verification

Renderer tests will first assert that the deleted labels, element IDs, navigation target, and section are absent while the four remaining sections stay in order. Static-resource assertions will prevent the deleted controls from returning. The full report test suite will then run.

Finally, `deepseek-v4-flash-model-capability-report.html` will be regenerated from `deepseek-v4-flash-assessment.json` through the renderer, then checked for the same absence and section-order conditions. A browser screenshot will confirm the sidebar ends after the four navigation links and the report ends after the capability results.

## Non-Goals

This change does not alter assessment JSON, evaluation rules, verdict logic, evidence content, capability filters, or expandable detail rows.
