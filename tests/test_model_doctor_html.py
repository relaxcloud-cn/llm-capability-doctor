import re
import sys
import unittest
from copy import deepcopy
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SKILL_ROOT = ROOT / "skills" / "creating-model-doctor-reports"
SCRIPT_DIR = SKILL_ROOT / "scripts"
ASSET_DIR = SKILL_ROOT / "assets"
FIXTURES = ROOT / "tests" / "fixtures" / "model-doctor"
sys.path.insert(0, str(SCRIPT_DIR))

from model_doctor_assessment import assemble_assessment  # noqa: E402
from model_doctor_html import render_report  # noqa: E402
from model_doctor_log import parse_log  # noqa: E402


def review(status="PASS"):
    return {
        "testId": "001",
        "reviewedStatus": status,
        "confidence": "high",
        "gateLevel": "critical",
        "conclusion": "完整 URL 可连接并正常返回。",
        "logic": {
            "purpose": "验证完整模型地址是否可连接。",
            "method": "发送最小生成请求并检查传输与响应。",
            "passCriteria": ["curl 成功且 HTTP 为 2xx。"],
            "failCriteria": ["连接失败或接口未返回响应。"],
            "capabilityBoundary": "不证明上游商业模型身份。",
        },
        "evidenceRefs": ["request:test-001"],
        "evidenceExcerpts": ["HTTP 200，响应正文非空。"],
        "limitations": [],
        "retestInstructions": [],
    }


def assessment(status="PASS"):
    parsed = parse_log(FIXTURES / "minimal.log")
    return assemble_assessment(parsed, {"001": review(status)})


class ModelDoctorHtmlTests(unittest.TestCase):
    def test_render_report_has_decision_first_information_architecture(self):
        html = render_report(assessment(), ASSET_DIR)

        self.assertIn('<html lang="zh-CN">', html)
        self.assertIn("Model Doctor 客户模型就绪度报告", html)
        self.assertIn("READY", html)
        self.assertIn("test-model", html)
        self.assertIn("openai_chat", html)
        self.assertLess(html.index("总体结论"), html.index("能力域总结"))
        self.assertLess(html.index("能力域总结"), html.index("检测方法"))
        self.assertLess(html.index("检测方法"), html.index("逐项检测结果"))

    def test_render_report_shows_logic_input_output_and_evidence(self):
        html = render_report(assessment(), ASSET_DIR)

        for text in (
            "检测目的",
            "测试方法",
            "通过条件",
            "失败条件",
            "能力边界",
            "请求输入",
            "模型输出",
            "判定证据",
            "Reply only OK",
            'content&quot;:&quot;OK',
        ):
            self.assertIn(text, html)

    def test_render_report_is_offline_and_renders_hostile_output_as_text(self):
        value = assessment()
        value["tests"][0]["requests"][0]["responseBody"] = '<script>alert("x")</script><img src=x onerror=alert(1)>'
        html = render_report(value, ASSET_DIR)

        self.assertNotRegex(html, r'(?:src|href)=["\']https?://')
        self.assertIn("default-src &#x27;none&#x27;", html)
        self.assertIn("connect-src &#x27;none&#x27;", html)
        self.assertIn("&lt;script&gt;alert", html)
        self.assertNotIn('<script>alert("x")</script>', html)
        self.assertNotIn("innerHTML", html)

    def test_render_report_shows_raw_and_reviewed_discrepancy(self):
        value = assessment("FAIL")
        html = render_report(value, ASSET_DIR)

        self.assertIn("原始判断", html)
        self.assertIn("Skill 复核", html)
        self.assertIn("判定发生变化", html)
        self.assertIn("BLOCKED", html)

    def test_render_report_has_grouping_filters_expand_controls_and_print_styles(self):
        html = render_report(assessment(), ASSET_DIR)

        self.assertIn('data-filter="status"', html)
        self.assertIn('data-filter="gate"', html)
        self.assertIn('data-filter="discrepancy"', html)
        self.assertIn("接口与协议", html)
        self.assertIn("展开全部证据", html)
        self.assertIn("@media print", html)
        self.assertIn("position: sticky", html)
        self.assertNotIn("max-width: 736px", html)

    def test_render_report_status_counts_match_assessment(self):
        value = assessment()
        html = render_report(value, ASSET_DIR)

        match = re.search(r'data-status-count="PASS"[^>]*>([0-9]+)<', html)
        self.assertIsNotNone(match)
        self.assertEqual(match.group(1), "1")

    def test_render_report_does_not_mutate_assessment(self):
        value = assessment()
        before = deepcopy(value)

        render_report(value, ASSET_DIR)

        self.assertEqual(value, before)


if __name__ == "__main__":
    unittest.main()
