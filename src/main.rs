//! Rust Clawdbot 主入口

use clap::{Parser, Subcommand};
use tracing::{error, info, Level};
use tracing_subscriber::FmtSubscriber;

use clawdbot::service::{ClawdbotService, ServiceConfig};

#[derive(Parser, Debug)]
#[command(name = "clawdbot")]
#[command(author = "Clawdbot Authors")]
#[command(version = "0.1.0")]
#[command(about = "一个高性能跨平台 AI 消息机器人", long_about = None)]
struct Args {
    #[arg(short, long, default_value = "clawdbot.toml")]
    config: String,

    #[arg(short, long)]
    verbose: bool,

    #[arg(short, long, default_value = "8080")]
    port: u16,

    /// Web 管理界面端口（0 表示不启动）
    #[arg(long, default_value = "3000")]
    web_port: u16,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    Start,
    Check,
    Version,
}

#[tokio::main]
async fn main() {
    // 加载 .env 文件
    dotenv::dotenv().ok();

    let args = Args::parse();

    let log_level = if args.verbose {
        Level::DEBUG
    } else {
        Level::INFO
    };

    let subscriber = FmtSubscriber::builder()
        .with_max_level(log_level)
        .finish();
    tracing::subscriber::set_global_default(subscriber)
        .expect("设置日志 subscriber 失败");

    info!(version = "0.1.0", "Rust Clawdbot 启动");

    match args.command {
        Some(Commands::Start) => {
            run_service(&args.config, args.port, args.web_port).await;
        }
        Some(Commands::Check) => {
            check_config(&args.config).await;
        }
        Some(Commands::Version) => {
            println!("Rust Clawdbot v0.1.0");
        }
        None => {
            run_service(&args.config, args.port, args.web_port).await;
        }
    }
}

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
