//! Clawdbot 主入口

use clap::{Parser, Subcommand};
use tracing::{error, info, Level};
use tracing_subscriber::FmtSubscriber;

use clawdbot::service::{ClawdbotService, ServiceConfig};

// 命令行参数解析结构体
#[derive(Parser, Debug)]
#[command(name = "clawdbot")]
#[command(author = "Yang Hao <apprank@outlook.com>")]
#[command(version = "0.1.0")]
#[command(about = "一个高性能跨平台 AI 消息机器人", long_about = None)]
struct Args {
    /// 配置文件路径
    #[arg(short, long, default_value = "clawdbot.toml")]
    config: String,

    /// 是否启用 verbose 模式（显示 DEBUG 日志）
    #[arg(short, long)]
    verbose: bool,

    /// 监听端口
    #[arg(short, long, default_value = "8080")]
    port: u16,

    /// Web 管理界面端口（0 表示不启动）
    #[arg(long, default_value = "3000")]
    web_port: u16,

    /// 子命令
    #[command(subcommand)]
    command: Option<Commands>,
}

// 子命令枚举
#[derive(Subcommand, Debug)]
enum Commands {
    /// 启动 Clawdbot 服务
    Start,
    /// 检查配置文件是否有效
    Check,
    /// 显示版本信息    
    Version,
}

// 主函数
#[tokio::main]
async fn main() {
    // 加载 .env 文件
    dotenv::dotenv().ok();

    let args = Args::parse();

    // 设置日志级别
    let log_level = if args.verbose {
        Level::DEBUG
    } else {
        Level::INFO
    };

    // 设置全局日志 subscriber
    let subscriber = FmtSubscriber::builder()
        .with_max_level(log_level)
        .finish();
    tracing::subscriber::set_global_default(subscriber)
        .expect("设置日志 subscriber 失败");

    info!(version = "0.1.0", "Clawdbot 启动");

    // 根据子命令执行不同操作
    match args.command {
        Some(Commands::Start) => {
            run_service(&args.config, args.port, args.web_port).await;
        }
        Some(Commands::Check) => {
            check_config(&args.config).await;
        }
        Some(Commands::Version) => {
            println!("Clawdbot v0.1.0");
        }
        None => {
            run_service(&args.config, args.port, args.web_port).await;
        }
    }
}

// 启动 Clawdbot 服务
async fn run_service(config_path: &str, port: u16, web_port: u16) {
    info!(path = config_path, port = port, web_port = web_port, "开始启动机器人");

    let service_config = ServiceConfig {
        config_path: config_path.to_string(),
        port,
        ..Default::default()
    };

    let mut service = ClawdbotService::new(service_config);

    if let Err(e) = service.initialize(config_path).await {
        error!(error = %e, "服务初始化失败");
        return;
    }

    // 启动 Web 服务器（如果指定了端口）
    if web_port > 0 {
        let web_server = clawdbot::web::WebServer::new(web_port);
        // TODO: 从服务状态中获取配置信息
        let web_state = web_server.state().clone();

        // 在后台启动 Web 服务器
        tokio::spawn(async move {
            if let Err(e) = web_server.start().await {
                error!(error = %e, "Web 服务器启动失败");
            }
        });

        info!(port = web_port, "Web 管理界面已启动: http://localhost:{}", web_port);
    }

    if let Err(e) = service.start().await {
        error!(error = %e, "服务运行出错");
    }

    info!("服务退出");
}

// 检查配置文件是否有效
async fn check_config(config_path: &str) {
    println!("验证配置文件: {}", config_path);

    let loader = clawdbot::infra::config::ConfigLoader::new();

    match loader.load(config_path).await {
        Ok(config) => {
            println!("配置验证成功!");
            println!("- AI Providers: {}", config.ai.providers.len());
            println!("- Channels: {}", config.channels.len());
        }
        Err(e) => {
            println!("配置验证失败: {}", e);
        }
    }
}
