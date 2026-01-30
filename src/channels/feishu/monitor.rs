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
        let response = client.get_messages(config.page_size).await?;

        let items = response.data.map(|d| d.items).unwrap_or_default();

        for item in items {
            let message_id = item.message_id.clone();
            let now = Instant::now();

            // 清理过期条目（超过5分钟）
            processed_messages.retain(|_, &mut v| now.duration_since(v) < Duration::from_secs(300));

            if processed_messages.contains_key(&message_id) {
                continue;
            }

            processed_messages.insert(message_id.clone(), now);

            // 安全获取可选字段
            let sender = item.sender.as_ref();
            let sender_id = sender.map(|s| &s.sender_id);
            let body = item.body.as_ref();

            // 构建事件请求
            let event_request = FeishuEventRequest {
                event_type: "im.message.receive_v1".to_string(),
                event_id: item.message_id.clone(),
                created_at: chrono::Utc::now().timestamp(),
                event: serde_json::json!({
                    "message": {
                        "message_id": item.message_id,
                        "root_id": item.root_id,
                        "parent_id": item.parent_id,
                        "msg_type": item.msg_type,
                        "chat_id": item.chat_id.unwrap_or_default(),
                        "sender": {
                            "sender_id": {
                                "open_id": sender_id.and_then(|s| s.open_id.clone()),
                                "user_id": sender_id.and_then(|s| s.user_id.clone()),
                                "union_id": sender_id.and_then(|s| s.union_id.clone()),
                            },
                            "sender_type": sender.map(|s| s.sender_type.clone()).unwrap_or_default(),
                        },
                        "body": {
                            "content": body.map(|b| b.content.clone()).unwrap_or_default(),
                        },
                        "create_time": item.create_time,
                    }
                }),
            };

            match event_handler.handle(&event_request).await {
                Ok(result) => {
                    debug!(message_id = %result.message.id, "消息处理成功");

                    if result.need_read_receipt {
                        let _ = client.mark_message_read(&message_id).await;
                    }
                }
                Err(e) => {
                    error!(message_id = %message_id, error = %e, "消息处理失败");
                }
            }
        }

        Ok(())
    }
}
