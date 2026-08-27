use serde::Serialize;

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisRule {
    pub test_id: &'static str,
    pub pass_criteria: &'static str,
}

macro_rules! rule {
    ($id:literal, $criteria:literal) => {
        AnalysisRule {
            test_id: $id,
            pass_criteria: $criteria,
        }
    };
}

pub const RULES: [AnalysisRule; 42] = [
    rule!("001", "至少一个探测请求获得可观察的 HTTP 响应。"),
    rule!(
        "002",
        "至少一个响应严格匹配受支持协议的原生同步结构并含非空模型文本。"
    ),
    rule!("003", "至少一个协议探测通过鉴权且接受请求中的模型名。"),
    rule!("004", "原生非流式响应完整，模型可见文本包含指定 marker。"),
    rule!("005", "原生流事件可重建指定 marker。"),
    rule!("006", "流包含指定 marker、正常协议终止且完整结束。"),
    rule!(
        "007",
        "响应提供协议原生且类型有效的输入和输出 Token usage。"
    ),
    rule!("008", "畸形 JSON 请求获得结构化且可观察的错误响应。"),
    rule!("009", "模型只返回与要求完全相等的单个 JSON 对象。"),
    rule!(
        "010",
        "JSON 恰含 name=alpha、count=7、enabled=true 且类型正确。"
    ),
    rule!(
        "011",
        "嵌套对象、数组顺序和值及 null 类型全部准确且无额外内容。"
    ),
    rule!(
        "012",
        "单个 JSON 对象包含要求的 result.verdict、impact 和 nextMove。"
    ),
    rule!("013", "调查阶段正确引用存在的 EVID-001 证据对象。"),
    rule!(
        "018",
        "上下文按实测 Token 判定：校准请求成功返回 usage；最终被接受的最大容量探针的服务端 prompt tokens 不低于 124000（128K 要求留余量）为 PASS；主探针因超长被拒且搜索出的实测上限不足 124000 为 FAIL；超时、疑似截断或与上下文长度无关的错误写入 limitations，不算 FAIL。"
    ),
    rule!("019", "去除首尾空白后内容与指定 marker 完全相等。"),
    rule!("020", "严格输出三行 [BEGIN]、ALPHA|BETA|GAMMA、[END]。"),
    rule!(
        "022",
        "四字段 JSON 的时间、来源、动作及标签数组值和顺序完全正确。"
    ),
    rule!("024", "限长摘要包含要求的关键点且符合长度约束。"),
    rule!("031", "多轮修正后的最终内容严格为 NEW_STATE。"),
    rule!(
        "033",
        "low 和 high 两档 thinking 请求均被接受并产生模型可见回复。"
    ),
    rule!(
        "035",
        "存在独立 reasoning/thinking 块且最终内容严格为指定 marker。"
    ),
    rule!(
        "036",
        "流式响应中 reasoning/thinking 与最终答案分离且正常结束。"
    ),
    rule!(
        "038",
        "JSON 中 order、bTime 和 cTime 的顺序与时间计算完全正确。"
    ),
    rule!("040", "完整流中恰有一个 get_weather(city=Beijing) 调用。"),
    rule!("041", "候选工具中只选择天气工具且参数 city=Beijing。"),
    rule!("042", "不产生正式工具调用且最终文本严格为指定 marker。"),
    rule!(
        "043",
        "恰调用一次天气工具，必填参数、类型、枚举和值均正确且无额外字段。"
    ),
    rule!(
        "044",
        "恰调用 inspect_target，嵌套 host 和 port 参数结构和值正确。"
    ),
    rule!("045", "完整流中天气和时间两个调用相互独立且参数正确。"),
    rule!("046", "官方协议工具调用、关联结果和最终答案形成完整闭环。"),
    rule!("047", "先调用天气再调用时间，关联正确并返回最终 marker。"),
    rule!(
        "048",
        "关联天气结果后，最终答案保留 marker 和 WEATHER_SUNNY。"
    ),
    rule!("049", "超时后恰重试一次，关联成功结果并返回最终 marker。"),
    rule!("050", "十个候选工具中恰选择 get_weather(city=Beijing)。"),
    rule!("052", "非流式请求返回正确 marker 且首字节时间为有效正数。"),
    rule!(
        "053",
        "真实流式数据返回正确 marker 且首字节时间为有效正数。"
    ),
    rule!("054", "非流式请求返回正确 marker 且总耗时为有效正数。"),
    rule!("055", "五个样本全部成功且都返回精确 marker。"),
    rule!(
        "056",
        "五个重复样本均返回精确 marker 且指标完整有效，可计算 P50 和 P95 延迟。"
    ),
    rule!(
        "057",
        "每个并发波次样本均为 HTTP 2xx、协议有效、回复非空且指标有效。"
    ),
    rule!(
        "059",
        "experiment 的最终 JSON 恰含三个要求的中文安全业务字段和值；control 仅用于失败归因。"
    ),
    rule!(
        "060",
        "experiment 的最终 JSON 恰含三个要求的英文安全业务字段和值；control 仅用于失败归因。"
    ),
];

pub fn rule_for(test_id: &str) -> Option<&'static AnalysisRule> {
    RULES.iter().find(|rule| rule.test_id == test_id)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::catalog::CATALOG;

    #[test]
    fn every_catalog_check_has_one_rule() {
        let rule_ids: HashSet<_> = RULES.iter().map(|rule| rule.test_id).collect();
        let catalog_ids: HashSet<_> = CATALOG.iter().map(|test| test.id).collect();

        assert_eq!(RULES.len(), CATALOG.len());
        assert_eq!(rule_ids, catalog_ids);
    }
}
