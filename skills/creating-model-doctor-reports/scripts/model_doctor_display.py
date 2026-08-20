#!/usr/bin/env python3
"""Fixed customer-facing display names for the capability report.

The assessment JSON keeps the manifest names from the evidence log as audit
data.  This module maps the fixed test IDs to the plain-language domains and
names approved in the report design, so every contract version renders the
same customer vocabulary.  Unknown test IDs fall back to their manifest
values.
"""

from __future__ import annotations

from typing import Optional, Tuple


DISPLAY_DOMAINS: Tuple[str, ...] = (
    "接口连接与回答",
    "按指定格式输出",
    "长文本处理能力",
    "按要求回答与处理文本",
    "思考模式与逻辑任务",
    "工具调用与连续执行",
    "响应速度与短时稳定性",
    "安全业务场景可用性",
)

# test ID -> (display domain, display name)
TEST_DISPLAY: dict = {
    "001": ("接口连接与回答", "接口地址是否能访问"),
    "002": ("接口连接与回答", "识别接口使用的消息格式"),
    "003": ("接口连接与回答", "API Key 和模型名称是否被接受"),
    "004": ("接口连接与回答", "普通请求能否正常回答"),
    "005": ("接口连接与回答", "流式请求能否持续返回内容"),
    "006": ("接口连接与回答", "流式回答是否正常结束"),
    "007": ("接口连接与回答", "是否返回 Token 用量（模型计量单位）"),
    "008": ("接口连接与回答", "错误请求是否说明原因"),
    "009": ("按指定格式输出", "只返回 JSON（结构化数据）"),
    "010": ("按指定格式输出", "必填内容和数据类型是否正确"),
    "011": ("按指定格式输出", "多层内容、数组和空值是否正确"),
    "012": ("按指定格式输出", "业务结果的核心内容是否完整"),
    "013": ("按指定格式输出", "调查步骤与证据是否正确对应"),
    "014": ("长文本处理能力", "约 3.2 万字符的长文本请求（8K 级）"),
    "015": ("长文本处理能力", "约 6.4 万字符的长文本请求（16K 级）"),
    "016": ("长文本处理能力", "约 12.8 万字符的长文本请求（32K 级）"),
    "017": ("长文本处理能力", "约 25.6 万字符的长文本请求（64K 级）"),
    "018": ("长文本处理能力", "约 51.2 万字符的长文本请求（128K 级）"),
    "019": ("按要求回答与处理文本", "是否严格按指定文字回答"),
    "020": ("按要求回答与处理文本", "是否同时遵守多项格式要求"),
    "022": ("按要求回答与处理文本", "从文本中提取多个指定内容"),
    "024": ("按要求回答与处理文本", "在字数限制内保留重点"),
    "031": ("按要求回答与处理文本", "能否记住并采用最新修正"),
    "033": ("思考模式与逻辑任务", "是否接受低、高两档思考强度"),
    "035": ("思考模式与逻辑任务", "思考内容与最终答案是否分开"),
    "036": ("思考模式与逻辑任务", "流式返回是否区分思考和答案"),
    "038": ("思考模式与逻辑任务", "逻辑顺序和时间计算是否正确"),
    "040": ("工具调用与连续执行", "能否调用一个工具"),
    "041": ("工具调用与连续执行", "能否从多个工具中选对工具"),
    "042": ("工具调用与连续执行", "不需要工具时能否直接回答"),
    "043": ("工具调用与连续执行", "工具参数是否完整且类型正确"),
    "044": ("工具调用与连续执行", "多层工具参数是否正确"),
    "045": ("工具调用与连续执行", "能否同时调用两个工具"),
    "046": ("工具调用与连续执行", "工具调用格式是否符合官方要求"),
    "047": ("工具调用与连续执行", "能否连续调用多个工具"),
    "048": ("工具调用与连续执行", "能否正确使用工具返回的结果"),
    "049": ("工具调用与连续执行", "工具超时后能否正确重试"),
    "050": ("工具调用与连续执行", "能否从十个工具中选对工具"),
    "052": ("响应速度与短时稳定性", "普通请求首次返回内容所需时间（TTFB）"),
    "053": ("响应速度与短时稳定性", "流式请求首次返回内容所需时间（TTFB）"),
    "054": ("响应速度与短时稳定性", "完整回答所需时间"),
    "055": ("响应速度与短时稳定性", "连续 5 次请求是否都成功"),
    "056": ("响应速度与短时稳定性", "50% 和 95% 请求完成所需时间（P50/P95）"),
    "057": ("响应速度与短时稳定性", "4 到 32 路同时请求是否成功"),
    "059": ("安全业务场景可用性", "能否处理中文安全业务场景"),
    "060": ("安全业务场景可用性", "能否处理英文安全业务场景"),
}

FAILURE_KIND_LABELS = {
    "DIRECT": "实际结果没有完成要求",
    "CONTRACT_FACET": "有一部分要求没有满足",
    "MEASUREMENT_UNAVAILABLE": "本次收集的数据不够，无法计算",
    "EVIDENCE_GAP": "缺少足够证据，无法判断",
}

EVIDENCE_SUFFICIENCY_LABELS = {
    "SUFFICIENT": "现有记录足以确认本次结果",
    "LIMITED": "现有记录有限，只能支持部分判断",
    "INSUFFICIENT": "现有记录不足，不能支持可靠判断",
}

TIER_LABELS = {
    "core": "关键检查",
    "enhanced": "扩展检查",
}

PROTOCOL_FAMILY_LABELS = {
    "OPENAI_CHAT_COMPLETIONS": "OpenAI Chat Completions",
    "OPENAI_RESPONSES": "OpenAI Responses",
    "ANTHROPIC_MESSAGES": "Anthropic Messages",
    "GEMINI_GENERATE_CONTENT": "Gemini GenerateContent",
    "OLLAMA_CHAT": "Ollama Chat",
    "CUSTOM": "自定义格式",
    "UNKNOWN": "未确认",
}

def display_domain(test: dict) -> str:
    entry = TEST_DISPLAY.get(str(test.get("testId", "")))
    if entry:
        return entry[0]
    return str(test.get("category") or "未分类")


def display_name(test: dict) -> str:
    entry = TEST_DISPLAY.get(str(test.get("testId", "")))
    if entry:
        return entry[1]
    return str(test.get("name") or f"检测项 {test.get('testId', '未知')}")


def domain_order(domains_with_tests: list) -> list:
    """Order known display domains first, then any extra domains by name."""

    known = [name for name in DISPLAY_DOMAINS if name in domains_with_tests]
    extras = sorted(set(domains_with_tests) - set(DISPLAY_DOMAINS))
    return known + extras


def failure_kind_label(value: object) -> str:
    key = value if isinstance(value, str) else ""
    return FAILURE_KIND_LABELS.get(key, key or "未分类")


def sufficiency_label(value: object) -> str:
    key = value if isinstance(value, str) else ""
    return EVIDENCE_SUFFICIENCY_LABELS.get(key, key or "未分类")


def protocol_family_label(value: object, default: Optional[str] = None) -> str:
    if not isinstance(value, str):
        return default or "未确认"
    return PROTOCOL_FAMILY_LABELS.get(value, value)
