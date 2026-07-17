import json
import subprocess
import sys
import tempfile
import unittest
from copy import deepcopy
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SKILL_ROOT = ROOT / "skills" / "creating-model-doctor-reports"
SCRIPT_DIR = SKILL_ROOT / "scripts"
ASSET_DIR = SKILL_ROOT / "assets"
CLI = SCRIPT_DIR / "model_doctor_report.py"
FIXTURES = ROOT / "tests" / "fixtures" / "model-doctor"
sys.path.insert(0, str(SCRIPT_DIR))

from model_doctor_assessment import assemble_assessment  # noqa: E402
from model_doctor_html import render_report  # noqa: E402
from model_doctor_log import parse_log  # noqa: E402


FORBIDDEN_REPORT_TEXT = (
    "需复测",
    "待补证",
    "无法判定",
    "不支持",
    "跳过",
    "执行错误",
    "CONDITIONAL",
    "UNDETERMINED",
    "UNSUPPORTED",
    "SKIPPED",
    "ERROR",
)


def review(test_id, status, gate, evidence_refs):
    return {
        "testId": test_id,
        "reviewedStatus": status,
        "confidence": "high",
        "gateLevel": gate,
        "conclusion": (
            "完整证据满足检测要求。"
            if status == "PASS"
            else "完整证据未满足检测要求。"
        ),
        "logic": {
            "purpose": "验证目标能力。",
            "method": "检查 manifest 引用的完整请求与响应。",
            "passCriteria": ["可观察证据满足检测契约。"],
            "failCriteria": ["证据缺失、错误或与检测契约冲突。"],
            "capabilityBoundary": "仅证明本次可观察行为。",
        },
        "evidenceRefs": evidence_refs,
        "evidenceExcerpts": ["HTTP 和响应正文已核查。"],
        "limitations": [],
        "retestInstructions": [],
    }


def minimal_assessment(status="PASS"):
    parsed = parse_log(FIXTURES / "minimal.log")
    return assemble_assessment(
        parsed,
        {
            "001": review(
                "001",
                status,
                "critical",
                ["request:test-001"],
            )
        },
    )


def binary_assessment_fixture():
    parsed = parse_log(FIXTURES / "mixed.log")
    return assemble_assessment(
        parsed,
        {
            "047": review(
                "047",
                "PASS",
                "critical",
                ["request:test-047", "request:test-047-follow"],
            ),
            "056": review(
                "056",
                "FAIL",
                "observation",
                [
                    f"request:test-055-repeat-{index}"
                    for index in range(1, 6)
                ],
            ),
        },
    )


class ModelDoctorHtmlTests(unittest.TestCase):
    def test_report_is_binary_and_contains_manifest_requests(self):
        html = render_report(binary_assessment_fixture(), ASSET_DIR)

        self.assertIn("通过", html)
        self.assertIn("未通过", html)
        self.assertIn("test-047-follow", html)
        for forbidden in FORBIDDEN_REPORT_TEXT:
            self.assertNotIn(forbidden, html)

    def test_report_header_displays_run_metadata_with_escaping(self):
        value = minimal_assessment()
        value["run"].update(
            {
                "url": "https://model.example/v1/messages?region=cn&mode=<audit>",
                "model": "deepseek-v4-pro[1m]",
                "api_key": "sk-t********5678",
            }
        )

        html = render_report(value, ASSET_DIR)
        body = html.split("</style>", 1)[1]

        self.assertIn("检测信息", body)
        self.assertIn("检测 URL", body)
        self.assertIn("模型名称", body)
        self.assertIn("API Key", body)
        self.assertIn(
            "https://model.example/v1/messages?region=cn&amp;mode=&lt;audit&gt;",
            body,
        )
        self.assertIn("deepseek-v4-pro[1m]", body)
        self.assertIn("sk-t********5678", body)
        self.assertLess(body.index("检测信息"), body.index("能力域总结"))

    def test_render_report_uses_summary_and_two_priority_result_tables(self):
        html = render_report(binary_assessment_fixture(), ASSET_DIR)
        body = html.split("</style>", 1)[1]

        self.assertIn('<html lang="zh-CN">', html)
        self.assertIn("Model Doctor 客户模型就绪度报告", html)
        self.assertEqual(body.count("<table"), 3)
        self.assertIn("能力域总结", body)
        self.assertIn("重要检测项", body)
        self.assertIn("次要检测项", body)
        self.assertIn(
            "<th>能力域</th><th>状态</th><th>关键数据</th><th>最终结论</th>",
            body,
        )
        self.assertIn(
            "<th>编号</th><th>检测项</th><th>检测结果</th><th>检测结论</th>",
            body,
        )

    def test_result_rows_are_partitioned_by_gate_without_duplication(self):
        html = render_report(binary_assessment_fixture(), ASSET_DIR)
        body = html.split("</style>", 1)[1]
        important = body.split("重要检测项", 1)[1].split("次要检测项", 1)[0]
        secondary = body.split("次要检测项", 1)[1]

        self.assertIn('data-detail-id="test-detail-047"', important)
        self.assertNotIn('data-detail-id="test-detail-056"', important)
        self.assertIn('data-detail-id="test-detail-056"', secondary)
        self.assertEqual(body.count('data-detail-id="test-detail-047"'), 1)
        self.assertEqual(body.count('data-detail-id="test-detail-056"'), 1)

    def test_each_result_has_one_hidden_accessible_detail_row(self):
        html = render_report(minimal_assessment(), ASSET_DIR)

        self.assertIn('class="result-row" data-detail-id="test-detail-001"', html)
        self.assertIn(
            'class="row-toggle" aria-expanded="false" aria-controls="test-detail-001"',
            html,
        )
        self.assertIn('id="test-detail-001" class="evidence-row" hidden', html)
        self.assertIn('<td colspan="4">', html)

    def test_expanded_rows_pair_every_manifest_request_input_and_output(self):
        html = render_report(binary_assessment_fixture(), ASSET_DIR)

        self.assertEqual(html.count("请求输入"), 7)
        self.assertEqual(html.count("请求输出"), 7)
        self.assertLess(html.index("test-047"), html.index("test-047-follow"))
        self.assertIn("Call get_weather", html)
        self.assertIn("WEATHER_SUNNY", html)

    def test_expanded_row_contains_approved_logic_and_io_sections(self):
        html = render_report(minimal_assessment(), ASSET_DIR)

        for expected in ("检测目的", "检测方法", "通过条件", "请求输入", "请求输出"):
            self.assertIn(expected, html)
        self.assertIn("Reply only OK", html)
        self.assertIn('content&quot;:&quot;OK', html)

    def test_render_report_is_offline_and_renders_hostile_output_as_text(self):
        value = minimal_assessment()
        value["tests"][0]["requests"][0]["responseBody"] = (
            '<script>alert("x")</script><img src=x onerror=alert(1)>'
        )
        html = render_report(value, ASSET_DIR)

        self.assertNotRegex(html, r'(?:src|href)=["\']https?://')
        self.assertIn("default-src &#x27;none&#x27;", html)
        self.assertIn("connect-src &#x27;none&#x27;", html)
        self.assertIn("&lt;script&gt;alert", html)
        self.assertNotIn('<script>alert("x")</script>', html)
        self.assertNotIn("innerHTML", html)

    def test_fail_rows_and_categories_use_unambiguous_label(self):
        html = render_report(minimal_assessment("FAIL"), ASSET_DIR)

        self.assertIn('class="result-group" data-status="FAIL"', html)
        self.assertIn(
            '<span class="status status-FAIL">未通过</span>',
            html,
        )
        self.assertIn(
            '<span class="category-status category-status-FAIL">未通过</span>',
            html,
        )

    def test_category_key_data_lists_all_failed_test_ids(self):
        html = render_report(binary_assessment_fixture(), ASSET_DIR)

        self.assertIn("0/1 通过；未通过：056", html)
        self.assertIn("1/1 通过", html)

    def test_report_css_keeps_responsive_contract_without_old_statuses(self):
        html = render_report(minimal_assessment(), ASSET_DIR)

        for expected in (
            "max-width: 736px",
            "border-collapse: collapse",
            "prefers-color-scheme: dark",
            "@media (max-width: 640px)",
            ".evidence-row[hidden]",
            "overflow-wrap: anywhere",
        ):
            self.assertIn(expected, html)
        for forbidden in (
            ".status-UNDETERMINED",
            ".status-UNSUPPORTED",
            ".status-SKIPPED",
            ".status-ERROR",
            ".category-status-CONDITIONAL",
        ):
            self.assertNotIn(forbidden, html)

    def test_report_script_toggles_detail_and_aria_without_inner_html(self):
        html = render_report(minimal_assessment(), ASSET_DIR)

        for expected in (
            '.querySelectorAll(".result-row")',
            'button.setAttribute("aria-expanded"',
            "detail.hidden = !expanded",
            'row.classList.toggle("is-expanded"',
        ):
            self.assertIn(expected, html)
        self.assertNotIn("innerHTML", html)

    def test_render_report_does_not_mutate_assessment(self):
        value = binary_assessment_fixture()
        before = deepcopy(value)

        render_report(value, ASSET_DIR)

        self.assertEqual(value, before)


class ModelDoctorCliHtmlTests(unittest.TestCase):
    def test_render_command_writes_assessment_and_html_without_overwriting(self):
        with tempfile.TemporaryDirectory() as temporary_directory:
            directory = Path(temporary_directory)
            parsed_path = directory / "parsed.json"
            reviews_path = directory / "reviews.json"
            assessment_path = directory / "assessment.json"
            html_path = directory / "customer-readiness-report.html"
            parsed_path.write_text(
                json.dumps(parse_log(FIXTURES / "minimal.log")),
                encoding="utf-8",
            )
            reviews_path.write_text(
                json.dumps(
                    {
                        "001": review(
                            "001",
                            "PASS",
                            "critical",
                            ["request:test-001"],
                        )
                    }
                ),
                encoding="utf-8",
            )
            assessment_path.write_text("old assessment", encoding="utf-8")
            html_path.write_text("old report", encoding="utf-8")

            result = subprocess.run(
                [
                    sys.executable,
                    str(CLI),
                    "render",
                    str(parsed_path),
                    str(reviews_path),
                    "--assessment",
                    str(assessment_path),
                    "--html",
                    str(html_path),
                ],
                cwd=ROOT,
                capture_output=True,
                text=True,
                check=False,
            )

            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(
                assessment_path.read_text(encoding="utf-8"),
                "old assessment",
            )
            self.assertEqual(html_path.read_text(encoding="utf-8"), "old report")
            generated_paths = [
                Path(line) for line in result.stdout.splitlines() if line.strip()
            ]
            self.assertEqual(len(generated_paths), 2)
            self.assertTrue(
                all(path.is_absolute() and path.exists() for path in generated_paths)
            )
            generated_assessment = next(
                path for path in generated_paths if path.suffix == ".json"
            )
            generated_html = next(
                path for path in generated_paths if path.suffix == ".html"
            )
            self.assertEqual(
                json.loads(
                    generated_assessment.read_text(encoding="utf-8")
                )["overall"]["verdict"],
                "READY",
            )
            generated_html_text = generated_html.read_text(encoding="utf-8")
            self.assertIn("请求输入", generated_html_text)
            self.assertIn("请求输出", generated_html_text)


if __name__ == "__main__":
    unittest.main()
