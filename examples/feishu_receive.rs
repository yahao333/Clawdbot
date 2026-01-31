//! 飞书消息接收测试
//!
//! 运行此测试来验证飞书机器人接收消息功能
//!
//! # 使用方法
//! ```bash
//! cargo run --example feishu_receive --features feishu
//! ```
//!
//! 这将启动 WebSocket 连接，接收飞书推送的消息事件。

use dotenv;
use tracing::{error, info, Level};
use tracing_subscriber;

use clawdbot::channels::feishu::{FeishuClient, FeishuCredentials, FeishuWsMonitor};
use clawdbot::channels::feishu::handlers::MessageEventHandler;
use clawdbot::core::message::MessageHandler;
use clawdbot::infra::error::Error;

/// 简单消息处理器（用于测试）
#[derive(Clone, Default)]
struct TestMessageHandler;

#[async_trait::async_trait]
impl MessageHandler for TestMessageHandler {
    async fn handle(
        &self,
        _context: &clawdbot::core::message::HandlerContext,
        message: clawdbot::core::message::types::InboundMessage,
    ) -> Result<(), Error> {
        info!(
            target: "feishu_recv",
            "=== 收到飞书消息 ===\n\
             message_id: {}\n\
             source: {:?}\n\
             sender_id: {}\n\
             chat_id: {}\n\
             chat_type: {}\n\
             content: {:?}\n",
            message.id,
            message.source,
            message.sender.id,
            message.target.id,
            message.target.target_type.unwrap_or_default(),
            message.content.text
        );

        // 如果有附件，打印附件信息
        if !message.content.attachments.is_empty() {
            info!(target: "feishu_recv", "附件数量: {}", message.content.attachments.len());
            for (i, attachment) in message.content.attachments.iter().enumerate() {
                info!(
                    target: "feishu_recv",
                    "附件[{}]: kind={:?}, url={}",
                    i, attachment.kind, attachment.url.as_deref().unwrap_or("N/A")
                );
            }
        }

        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<(), String> {
    // 初始化日志
    tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .init();

    // 加载环境变量
    dotenv::dotenv().ok();

    // 获取凭证
    let app_id = std::env::var("FEISHU_APP_ID")
        .map_err(|_| "请设置 FEISHU_APP_ID 环境变量")?;

    let app_secret = std::env::var("FEISHU_APP_SECRET")
        .map_err(|_| "请设置 FEISHU_APP_SECRET 环境变量")?;

    let credentials = FeishuCredentials {
        app_id,
        app_secret,
        verification_token: None,
        encrypt_key: None,
    };

    info!(target: "feishu_recv", "飞书凭证已加载: app_id = {}", credentials.app_id);

    // 创建飞书客户端（克隆凭证）
    let client = std::sync::Arc::new(FeishuClient::new(credentials.clone()));

    // 测试获取 WebSocket URL
    info!(target: "feishu_recv", "获取 WebSocket URL...");
    match client.get_websocket_url().await {
        Ok(url) => {
            info!(target: "feishu_recv", "WebSocket URL 获取成功: {}...", &url[..50.min(url.len())]);
        }
        Err(e) => {
            error!(target: "feishu_recv", "获取 WebSocket URL 失败: {}", e);
            return Err(format!("获取 WebSocket URL 失败: {}", e));
        }
    }

    // 创建消息事件处理器
    let event_handler = MessageEventHandler::new();

    // 创建测试消息处理器
    let message_handler = std::sync::Arc::new(TestMessageHandler::default());

    // 创建简单的 HandlerContext（使用默认配置）
    // 使用默认 AI 引擎（用于测试）
    let handler_context = clawdbot::core::message::HandlerContext::new(
        std::sync::Arc::new(clawdbot::infra::config::Config::default()),
        std::sync::Arc::new(clawdbot::core::message::MessageQueue::new(clawdbot::core::message::queue::QueueConfig::default())),
        std::sync::Arc::new(clawdbot::core::routing::DefaultRouter::new("default")) as std::sync::Arc<dyn clawdbot::core::routing::Router>,
        std::sync::Arc::new(clawdbot::core::agent::DefaultAiEngine::new()) as std::sync::Arc<dyn clawdbot::core::agent::AiEngine>,
        std::sync::Arc::new(clawdbot::channels::feishu::FeishuMessageSender::new(credentials.clone())) as std::sync::Arc<dyn clawdbot::core::message::sender::MessageSender>,
    );

    // 创建 WebSocket 监控器
    let monitor = FeishuWsMonitor::new(
        client.clone(),
        event_handler,
        handler_context,
        message_handler,
    );

    info!(target: "feishu_recv", "启动飞书 WebSocket 监控服务...");
    info!(target: "feishu_recv", "按 Ctrl+C 停止服务");

    // 启动监控服务
    match monitor.start().await {
        Ok(_handle) => {
            info!(target: "feishu_recv", "飞书 WebSocket 监控服务已启动");
            info!(target: "feishu_recv", "等待接收消息... (请给机器人发送消息来测试)");

            // 保持运行直到 Ctrl+C
            tokio::signal::ctrl_c().await.map_err(|e| format!("Ctrl+C 错误: {}", e))?;
            info!(target: "feishu_recv", "收到停止信号");
            monitor.stop();
        }
        Err(e) => {
            error!(target: "feishu_recv", "启动监控服务失败: {}", e);
            return Err(format!("启动监控服务失败: {}", e));
        }
    }

    info!(target: "feishu_recv", "测试完成");
    Ok(())
}
