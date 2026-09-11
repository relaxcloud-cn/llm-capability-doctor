use clap::Parser;
use llm_capability_doctor::ingress::{ConnectionConfig, redact_endpoint};

#[derive(Debug, Parser)]
#[command(
    name = "llm-capability-doctor",
    version,
    about = "检测模型服务能力并输出有证据的使用结论"
)]
struct Cli {
    /// 当前模型服务地址；输出时会自动脱敏。
    #[arg(long, value_name = "URL")]
    url: Option<String>,

    /// 客户配置的模型名称。
    #[arg(long, value_name = "MODEL")]
    model: Option<String>,

    /// API 密钥，仅用于后续请求，不会输出。
    #[arg(long, value_name = "KEY", hide = true)]
    api_key: Option<String>,
}

fn main() {
    let cli = Cli::parse();
    let _api_key = cli.api_key;

    if let (Some(endpoint), Some(model)) = (cli.url, cli.model) {
        let config = ConnectionConfig::new(endpoint, model);
        println!("模型：{}", config.model);
        println!("地址：{}", redact_endpoint(&config.endpoint));
        println!("检测：not_tested");
        println!("最近一次：not_tested");
    }
}
