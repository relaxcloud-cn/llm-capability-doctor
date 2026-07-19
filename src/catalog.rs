use thiserror::Error;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TestCase {
    pub id: &'static str,
    pub category: &'static str,
    pub name: &'static str,
}

macro_rules! test_case {
    ($id:literal, $category:literal, $name:literal) => {
        TestCase {
            id: $id,
            category: $category,
            name: $name,
        }
    };
}

pub static CATALOG: [TestCase; 62] = [
    test_case!("001", "接口与协议", "URL 可达性"),
    test_case!("002", "接口与协议", "协议识别"),
    test_case!("003", "接口与协议", "鉴权与模型接受"),
    test_case!("004", "接口与协议", "同步生成"),
    test_case!("005", "接口与协议", "流式生成"),
    test_case!("006", "接口与协议", "流结束完整性"),
    test_case!("007", "接口与协议", "Token usage"),
    test_case!("008", "接口与协议", "错误可观测性"),
    test_case!("009", "结构化结果", "裸 JSON 输出"),
    test_case!("010", "结构化结果", "必填字段与类型"),
    test_case!("011", "结构化结果", "嵌套数组与空值"),
    test_case!("012", "结构化结果", "Result 核心字段"),
    test_case!("013", "结构化结果", "调查阶段与证据引用"),
    test_case!("014", "上下文", "8K 级上下文（字符近似）"),
    test_case!("015", "上下文", "16K 级上下文（字符近似）"),
    test_case!("016", "上下文", "32K 级上下文（字符近似）"),
    test_case!("017", "上下文", "64K 级上下文（字符近似）"),
    test_case!("018", "上下文", "128K 级上下文（字符近似）"),
    test_case!("019", "指令与文本", "精确输出"),
    test_case!("020", "指令与文本", "组合格式约束"),
    test_case!("021", "指令与文本", "指令修正优先级"),
    test_case!("022", "指令与文本", "多字段抽取"),
    test_case!("023", "指令与文本", "多标签分类"),
    test_case!("024", "指令与文本", "限长摘要关键点"),
    test_case!("025", "指令与文本", "合并与去重"),
    test_case!("026", "上下文", "开头信息召回"),
    test_case!("027", "上下文", "中间信息召回"),
    test_case!("028", "上下文", "结尾信息召回"),
    test_case!("029", "上下文", "多标记跨段关联"),
    test_case!("030", "上下文", "相似干扰与噪声"),
    test_case!("031", "上下文", "多轮修正记忆"),
    test_case!("032", "Thinking 与推理", "Thinking 参数接受"),
    test_case!("033", "Thinking 与推理", "Thinking 档位接受"),
    test_case!("034", "Thinking 与推理", "Reasoning token"),
    test_case!("035", "Thinking 与推理", "思考与答案分离"),
    test_case!("036", "Thinking 与推理", "Thinking 流式事件"),
    test_case!("037", "Thinking 与推理", "多步计算"),
    test_case!("038", "Thinking 与推理", "逻辑与时序推理"),
    test_case!("039", "Thinking 与推理", "规划与复核"),
    test_case!("040", "工具调用", "单工具调用"),
    test_case!("041", "工具调用", "工具选择"),
    test_case!("042", "工具调用", "无需工具时不调用"),
    test_case!("043", "工具调用", "必填参数与类型枚举"),
    test_case!("044", "工具调用", "嵌套参数"),
    test_case!("045", "工具调用", "并行工具调用"),
    test_case!("046", "工具调用", "Call ID 完整性"),
    test_case!("047", "工具调用", "串行工具调用"),
    test_case!("048", "工具调用", "工具结果忠实性"),
    test_case!("049", "工具调用", "工具失败恢复"),
    test_case!("050", "工具调用", "大工具目录"),
    test_case!("051", "性能与稳定性", "冷请求总延迟"),
    test_case!("052", "性能与稳定性", "首字节时间"),
    test_case!("053", "性能与稳定性", "流式首字节时间"),
    test_case!("054", "性能与稳定性", "完整响应延迟"),
    test_case!("055", "性能与稳定性", "重复成功率"),
    test_case!("056", "性能与稳定性", "P50/P95 延迟"),
    test_case!("057", "性能与稳定性", "4-32 并发响应时间"),
    test_case!("058", "性能与稳定性", "持续请求与恢复探针"),
    test_case!("059", "护栏与词汇", "越权请求护栏"),
    test_case!("060", "护栏与词汇", "合法防御分析"),
    test_case!("061", "护栏与词汇", "中文安全词可用性"),
    test_case!("062", "护栏与词汇", "英文安全词可用性"),
];

#[derive(Debug, Error, Eq, PartialEq)]
pub enum CatalogError {
    #[error("Invalid --only test ID: {0}")]
    InvalidId(String),
    #[error("Unknown --only test ID: {0}")]
    UnknownId(String),
}

pub fn render() -> String {
    let mut output = String::new();
    for test in CATALOG {
        output.push_str(test.id);
        output.push('\t');
        output.push_str(test.category);
        output.push('\t');
        output.push_str(test.name);
        output.push('\n');
    }
    output
}

pub fn select(only: Option<&str>) -> Result<Vec<&'static TestCase>, CatalogError> {
    let Some(only) = only else {
        return Ok(CATALOG.iter().collect());
    };

    only.split(',')
        .map(|id| {
            if id.len() != 3 || !id.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(CatalogError::InvalidId(id.to_owned()));
            }
            CATALOG
                .iter()
                .find(|test| test.id == id)
                .ok_or_else(|| CatalogError::UnknownId(id.to_owned()))
        })
        .collect()
}
