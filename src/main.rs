//! Clawdbot 主入口

use std::sync::Arc;
use clap::{Parser, Subcommand};
use tracing::{error, info, warn, Level};
use tracing_subscriber::FmtSubscriber;

use clawdbot::service::{ClawdbotService, ServiceConfig};
use clawdbot::web::ServiceStatus;
use clawdbot::infra::config::ConfigLoader;
use clawdbot::security::AuditService;

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

    // 1. 先加载配置（需要用于初始化 AuditService）
    let loader = ConfigLoader::new();
    let config = match loader.load(config_path).await {
        Ok(cfg) => {
            info!("配置加载成功");
            cfg
        }
        Err(e) => {
            error!(error = %e, "配置加载失败");
            return;
        }
    };

    // 2. 如果启用了安全审计，创建 AuditService
    let audit_service = if config.security.enabled {
        match AuditService::new(Arc::new(config.clone())).await {
            Ok(service) => {
                info!("审计服务已启用");
                Some(service)
            }
            Err(e) => {
                warn!(error = %e, "审计服务初始化失败，将继续运行但不记录审计日志");
                None
            }
        }
    } else {
        info!("安全审计已禁用");
        None
    };

    let service_config = ServiceConfig {
        config_path: config_path.to_string(),
        port,
        ..Default::default()
    };

    // 3. 创建 Web 服务器（如果需要）
    let web_server = if web_port > 0 {
        Some(clawdbot::web::WebServer::new(web_port))
    } else {
        None
    };

    // 4. 获取 Web 状态（用于仪表盘数据更新）
    let web_state: Arc<clawdbot::web::WebState> = if let Some(ref ws) = web_server {
        ws.state().clone()
    } else {
        Arc::new(clawdbot::web::WebState::new())
    };

    let mut service = ClawdbotService::new(service_config, web_state);

    if let Err(e) = service.initialize(config_path).await {
        error!(error = %e, "服务初始化失败");
        return;
    }

    // 5. 设置审计服务到 WebServer（需要在服务初始化之后）
    if let Some(ref ws) = web_server {
        if let Some(audit_svc) = audit_service {
            ws.set_audit_service(audit_svc);
            info!("审计服务已绑定到 Web 管理界面");
        }
    }

    // 6. 启动 Web 服务器（如果指定了端口）
    if web_port > 0 {
        // 使用之前创建的 web_server，而不是创建新的实例
        let web_server = web_server.unwrap();  // 获取之前创建的 WebServer

        // 读取并应用 Web 认证配置
        let web_auth_enabled = std::env::var("WEB_AUTH_ENABLED")
            .unwrap_or_else(|_| "false".to_string())
            .to_lowercase() == "true";
        let web_auth_username = std::env::var("WEB_AUTH_USERNAME").unwrap_or_else(|_| "admin".to_string());
        let web_auth_password = std::env::var("WEB_AUTH_PASSWORD").unwrap_or_else(|_| "password".to_string());

        if web_auth_enabled {
            web_server.configure_auth(&web_auth_username, &web_auth_password);
            info!(username = web_auth_username, "Web 认证已启用");
        } else {
            web_server.disable_auth();
            info!("Web 认证已禁用");
        }

        // 设置服务状态为初始化中
        web_server.set_service_status(ServiceStatus::Initializing);

        // 在后台启动 Web 服务器
        let web_server_clone = web_server.clone();
        tokio::spawn(async move {
            // 设置服务状态为运行中
            web_server_clone.set_service_status(ServiceStatus::Running);

            // 启动 Web 服务器
            if let Err(e) = web_server_clone.start().await {
                error!(error = %e, "Web 服务器启动失败");
                // 设置服务状态为错误
                web_server_clone.set_service_status(ServiceStatus::Error(e.to_string()));
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
