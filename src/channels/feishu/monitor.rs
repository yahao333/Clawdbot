//! 飞书消息监控模块
//!
//! 本模块实现了长链接消息监控功能，用于通过轮询方式接收飞书消息。

use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::signal;
use tokio::sync::broadcast;
use tracing::{debug, error, info};

use super::client::FeishuClient;
use super::handlers::{FeishuEventRequest, FeishuMessageItem, MessageEventHandler};

/// 消息轮询配置
#[derive(Debug, Clone)]
pub struct PollingConfig {
    /// 轮询间隔（毫秒）
    pub interval_ms: u64,
    /// 每次拉取消息数量
    pub page_size: u32,
}

impl Default for PollingConfig {
    fn default() -> Self {
        Self {
            interval_ms: 500,
            page_size: 20,
        }
    }
}

/// 飞书消息监控器
///
/// 通过长链接轮询方式监控飞书消息
#[derive(Clone)]
pub struct FeishuMessageMonitor {
    /// 飞书客户端
    client: Arc<FeishuClient>,
    /// 消息事件处理器
    event_handler: Arc<MessageEventHandler>,
    /// 轮询配置
    polling_config: PollingConfig,
    /// 已处理的消息 ID 集合
    processed_messages: Arc<dashmap::DashMap<String, Instant>>,
    /// 停止发送器
    shutdown_tx: broadcast::Sender<()>,
}

impl FeishuMessageMonitor {
    /// 创建消息监控器
    pub fn new(
        client: Arc<FeishuClient>,
        event_handler: MessageEventHandler,
    ) -> Self {
        let (shutdown_tx, _) = broadcast::channel(1);

        Self {
            client,
            event_handler: Arc::new(event_handler),
            polling_config: PollingConfig::default(),
            processed_messages: Arc::new(dashmap::DashMap::new()),
            shutdown_tx,
        }
    }

    /// 启动监控服务
    pub async fn start(&self) -> Result<tokio::task::JoinHandle<()>, crate::infra::error::Error> {
        info!("启动飞书消息监控服务（长链接轮询）");

        let client = self.client.clone();
        let event_handler = self.event_handler.clone();
        let polling_config = self.polling_config.clone();
        let processed_messages = self.processed_messages.clone();
        let mut shutdown_rx = self.shutdown_tx.subscribe();

        let interval_ms = self.polling_config.interval_ms;
        let handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(polling_config.interval_ms));

            loop {
                tokio::select! {
                    _ = shutdown_rx.recv() => {
                        info!("收到停止信号，退出消息监控");
                        break;
                    }
                    _ = interval.tick() => {
                        if let Err(e) = Self::poll_messages(
                            &client,
                            &event_handler,
                            &polling_config,
                            &processed_messages,
                        ).await {
                            error!(error = %e, "轮询消息失败");
                        }
                    }
                }
            }
        });

        let shutdown_tx = self.shutdown_tx.clone();
        tokio::spawn(async move {
            let _ = signal::ctrl_c().await;
            info!("收到 Ctrl+C 信号，停止消息监控");
            let _ = shutdown_tx.send(());
        });

        info!("飞书消息监控服务已启动（轮询间隔: {}ms)", interval_ms);

        Ok(handle)
    }

    /// 停止监控服务
    pub fn stop(&self) {
        let _ = self.shutdown_tx.send(());
    }

    /// 轮询获取消息
    async fn poll_messages(
        client: &FeishuClient,
        event_handler: &MessageEventHandler,
        config: &PollingConfig,
        processed_messages: &Arc<dashmap::DashMap<String, Instant>>,
    ) -> Result<(), crate::infra::error::Error> {
        // 注意：飞书 API (GET /im/v1/messages) 要求必须提供 container_id (chat_id)。
        // 这里的轮询实现因缺少 container_id 而无法工作。
        // 暂时禁用。
        
        // let response = client.get_messages("chat", "SOME_CHAT_ID", config.page_size).await?;
        
        // 消除 unused 警告
        let _ = client;
        let _ = event_handler;
        let _ = config;
        let _ = processed_messages;
        
        Ok(())

        /*
        let response = client.get_messages(config.page_size).await?;

        let items = response.data.map(|d| d.items).unwrap_or_default();

        for item in items {
            // ... (原有逻辑)
        }
        */
    }
}
