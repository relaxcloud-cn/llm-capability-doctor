use std::path::Path;

pub fn display_category(category: &str) -> &str {
    match category {
        "上下文" => "上下文能力",
        other => other,
    }
}

pub fn display_title(test_id: &str, concurrency: Option<usize>) -> String {
    match test_id {
        "001" => "HTTP 接口连通性与可观察响应",
        "002" => "模型接口协议识别与响应结构",
        "003" => "鉴权有效性与模型名称接受",
        "004" => "同步文本生成响应",
        "005" => "流式文本生成响应",
        "006" => "流式响应正常结束",
        "007" => "用量统计字段返回",
        "008" => "异常请求错误信息可观测性",
        "009" => "严格裸 JSON 输出",
        "010" => "JSON 必填字段与数据类型",
        "011" => "JSON 嵌套对象、数组与空值",
        "012" => "结构化结果核心字段",
        "013" => "调查阶段与证据引用结构",
        "018" => "实测模型可用上下文 Token 上限",
        "019" => "精确文本输出一致性",
        "020" => "多行组合格式约束",
        "022" => "多字段信息抽取",
        "024" => "限长摘要关键要点保留",
        "031" => "多轮修正后的上下文记忆",
        "033" => "推理档位参数接受度",
        "035" => "推理内容与最终答案分离",
        "036" => "流式推理事件与最终答案分离",
        "038" => "逻辑关系与时间顺序推理",
        "040" => "单工具调用结构",
        "041" => "多工具候选下的正确工具选择",
        "042" => "无需工具时不发起工具调用",
        "043" => "工具必填参数、数据类型与枚举值",
        "044" => "工具嵌套参数结构",
        "045" => "并行工具调用能力",
        "046" => "官方工具协议结构与结果闭环",
        "047" => "连续工具调用与结果关联",
        "048" => "工具结果内容保留",
        "049" => "工具调用失败后的恢复",
        "050" => "大工具目录下的工具选择",
        "052" => "非流式首字节响应时间",
        "053" => "流式首字节响应时间",
        "054" => "完整响应时间",
        "055" => "重复请求成功率",
        "056" => "P50/P95 响应时间",
        "057" => match concurrency {
            Some(level @ (4 | 8 | 16 | 32)) => {
                return format!("测试模型在 {level} 并发下能否正常响应");
            }
            _ => "4/8/16/32 并发响应表现",
        },
        "059" => "中文安全业务词与业务字段可用性",
        "060" => "英文安全业务词与业务字段可用性",
        _ => "未命名检测项",
    }
    .to_owned()
}

pub fn collection_line(
    position: usize,
    total: usize,
    category: &str,
    title: &str,
    wave: Option<(usize, usize)>,
) -> String {
    let progress = match wave {
        Some((current, wave_total)) => {
            format!("采集 {position}/{total} | 并发 {current}/{wave_total}")
        }
        None => format!("采集 {position}/{total}"),
    };
    format!("[{progress}] {} | {title}", display_category(category))
}

pub fn analysis_start_line(
    index: usize,
    total: usize,
    categories: &[String],
    test_ids: &[String],
) -> String {
    format!(
        "[自分析 {index:02}/{total:02}] {} | {} | 正在分析",
        categories.join("、"),
        format_test_ids(test_ids)
    )
}

pub fn analysis_retry_line(
    index: usize,
    total: usize,
    categories: &[String],
    retry: usize,
    max_retries: usize,
) -> String {
    format!(
        "[自分析 {index:02}/{total:02}] {} | 重试 {retry}/{max_retries}",
        categories.join("、")
    )
}

pub fn analysis_finish_line(
    index: usize,
    total: usize,
    categories: &[String],
    unavailable: bool,
) -> String {
    let state = if unavailable {
        "分析不可用"
    } else {
        "已完成"
    };
    format!(
        "[自分析 {index:02}/{total:02}] {} | {state}",
        categories.join("、")
    )
}

pub fn display_path(path: &Path) -> String {
    let Ok(current_directory) = std::env::current_dir() else {
        return path.display().to_string();
    };
    match path.strip_prefix(current_directory) {
        Ok(relative) if relative.as_os_str().is_empty() => "./".into(),
        Ok(relative) => format!("./{}", relative.display()),
        Err(_) => path.display().to_string(),
    }
}

fn format_test_ids(test_ids: &[String]) -> String {
    let Some(first) = test_ids.first() else {
        return "无检测项".into();
    };
    let consecutive = test_ids
        .windows(2)
        .all(|pair| numeric_test_id(&pair[1]) == numeric_test_id(&pair[0]).map(|id| id + 1));
    if test_ids.len() > 1 && consecutive {
        format!("{}-{}", first, test_ids.last().expect("non-empty IDs"))
    } else {
        test_ids.join("、")
    }
}

fn numeric_test_id(value: &str) -> Option<u16> {
    value.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analysis_lines_are_plain_text_and_compress_contiguous_test_ids() {
        let categories = vec!["接口与协议".into()];
        let test_ids = vec!["001".into(), "002".into(), "003".into(), "004".into()];

        assert_eq!(
            analysis_start_line(1, 12, &categories, &test_ids),
            "[自分析 01/12] 接口与协议 | 001-004 | 正在分析"
        );
        assert_eq!(
            analysis_retry_line(1, 12, &categories, 1, 3),
            "[自分析 01/12] 接口与协议 | 重试 1/3"
        );
        assert_eq!(
            analysis_finish_line(1, 12, &categories, true),
            "[自分析 01/12] 接口与协议 | 分析不可用"
        );
    }
}
