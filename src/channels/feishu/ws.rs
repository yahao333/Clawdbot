//! 飞书 WebSocket 长链接模块
//!
//! 实现飞书消息的 WebSocket 长连接接收。

use std::sync::Arc;
use std::time::{Duration, Instant};
use serde::Deserialize;
use tokio::signal;
use tokio::sync::broadcast;
use tracing::{debug, error, info, warn};

use super::client::FeishuClient;
use super::handlers::{FeishuEventRequest, MessageEventHandler};
use crate::core::message::{HandlerContext, MessageHandler};

/// WebSocket 配置
#[derive(Debug, Clone)]
pub struct WsConfig {
    /// 重连间隔（秒）
    pub reconnect_interval_secs: u64,
    /// 心跳间隔（秒）
    pub heartbeat_interval_secs: u64,
    /// 读取超时（秒）
    pub read_timeout_secs: u64,
}

impl Default for WsConfig {
    fn default() -> Self {
        Self {
            reconnect_interval_secs: 5,
            heartbeat_interval_secs: 30,
            read_timeout_secs: 60,
        }
    }
}

/// 飞书 WebSocket 消息
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub enum WsMessage {
    /// 发送数据
    #[serde(rename = "send")]
    Send {
        #[serde(default)]
        id: String,
        #[serde(default)]
        content: serde_json::Value,
    },
    /// 发送数据响应
    #[serde(rename = "send_ack")]
    SendAck {
        #[serde(default)]
        id: String,
        code: i64,
        msg: String,
    },
    /// 收到数据
    #[serde(rename = "receive")]
    Receive {
        #[serde(default)]
        id: String,
        #[serde(default)]
        content: serde_json::Value,
    },
    /// 错误
    #[serde(rename = "error")]
    Error {
        #[serde(default)]
        id: String,
        code: i64,
        msg: String,
    },
    /// 未知
    #[serde(other)]
    Unknown,
}

/// 飞书 WebSocket 监控器
///
/// 通过 WebSocket 长连接接收飞书消息
#[derive(Clone)]
pub struct FeishuWsMonitor {
    /// 飞书客户端
    client: Arc<FeishuClient>,
    /// 消息事件处理器
    event_handler: Arc<MessageEventHandler>,
    /// 核心消息处理上下文
    handler_context: Arc<HandlerContext>,
    /// 核心消息处理器
    message_handler: Arc<dyn MessageHandler>,
    /// WebSocket 配置
    ws_config: WsConfig,
    /// 已处理的消息 ID 集合
    processed_messages: Arc<dashmap::DashMap<String, Instant>>,
    /// 停止发送器
    shutdown_tx: broadcast::Sender<()>,
}

impl FeishuWsMonitor {
    /// 创建 WebSocket 监控器
    pub fn new(
        client: Arc<FeishuClient>,
        event_handler: MessageEventHandler,
        handler_context: HandlerContext,
        message_handler: Arc<dyn MessageHandler>,
    ) -> Self {
        let (shutdown_tx, _) = broadcast::channel(1);

        Self {
            client,
            event_handler: Arc::new(event_handler),
            handler_context: Arc::new(handler_context),
            message_handler,
            ws_config: WsConfig::default(),
            processed_messages: Arc::new(dashmap::DashMap::new()),
            shutdown_tx,
        }
    }

    /// 启动监控服务
    pub async fn start(&self) -> Result<tokio::task::JoinHandle<()>, crate::infra::error::Error> {
        info!("启动飞书 WebSocket 监控服务（长链接）");

        let client = self.client.clone();
        let event_handler = self.event_handler.clone();
        let handler_context = self.handler_context.clone();
        let message_handler = self.message_handler.clone();
        let ws_config = self.ws_config.clone();
        let processed_messages = self.processed_messages.clone();
        // Clone sender to create receivers inside the task
        let shutdown_tx = self.shutdown_tx.clone();
        let reconnect_interval = self.ws_config.reconnect_interval_secs;

        let handle = tokio::spawn(async move {
            let mut shutdown_rx = shutdown_tx.subscribe();
            let mut ws_url = None;

            while shutdown_rx.try_recv().is_err() {
                // 获取 WebSocket URL
                if ws_url.is_none() {
                    match client.get_websocket_url().await {
                        Ok(url) => {
                            info!(url = %url, "获取 WebSocket URL 成功");
                            ws_url = Some(url);
                        }
                        Err(e) => {
                            error!(error = %e, "获取 WebSocket URL 失败");
                            tokio::time::sleep(Duration::from_secs(reconnect_interval)).await;
                            continue;
                        }
                    }
                }

                // 连接 WebSocket
                if let Some(url) = &ws_url {
                    match client.connect_websocket(url).await {
                        Ok((mut ws, _)) => {
                            info!("WebSocket 连接成功");

                            // 创建内部停止接收器
                            let mut inner_shutdown = shutdown_tx.subscribe();

                            // 启动心跳和读取任务
                            let heartbeat_interval = tokio::time::interval(Duration::from_secs(ws_config.heartbeat_interval_secs));
                            let read_timeout = Duration::from_secs(ws_config.read_timeout_secs);
                            let processed_messages = processed_messages.clone();
                            let event_handler = event_handler.clone();
                            let handler_context = handler_context.clone();
                            let message_handler = message_handler.clone();
                            let client = client.clone();

                            let (exit_tx, mut exit_rx) = tokio::sync::mpsc::channel(1);

                            // 读取任务
                            let read_task = tokio::spawn(async move {
                                Self::read_websocket(
                                    &mut ws,
                                    read_timeout,
                                    &processed_messages,
                                    &event_handler,
                                    &handler_context,
                                    message_handler.as_ref(),
                                    heartbeat_interval,
                                    &client,
                                    &mut inner_shutdown,
                                    &exit_tx,
                                ).await;
                            });

                            // 等待退出信号
                            let _ = exit_rx.recv().await;

                            // 停止读取任务
                            read_task.abort();
                        }
                        Err(e) => {
                            error!(error = %e, "WebSocket 连接失败");
                            ws_url = None;
                            tokio::time::sleep(Duration::from_secs(reconnect_interval)).await;
                        }
                    }
                }

                // 短暂休眠后重试
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        });

        // Ctrl+C 处理
        let shutdown_tx = self.shutdown_tx.clone();
        tokio::spawn(async move {
            let _ = signal::ctrl_c().await;
            info!("收到 Ctrl+C 信号，停止 WebSocket 监控");
            let _ = shutdown_tx.send(());
        });

        info!("飞书 WebSocket 监控服务已启动");

        Ok(handle)
    }

    /// 停止监控服务
    pub fn stop(&self) {
        let _ = self.shutdown_tx.send(());
    }

    /// 读取和处理 WebSocket 消息
    async fn read_websocket(
        ws: &mut tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>
        >,
        read_timeout: Duration,
        processed_messages: &Arc<dashmap::DashMap<String, Instant>>,
        event_handler: &MessageEventHandler,
        handler_context: &HandlerContext,
        message_handler: &dyn MessageHandler,
        mut heartbeat_interval: tokio::time::Interval,
        client: &FeishuClient,
        shutdown_rx: &mut tokio::sync::broadcast::Receiver<()>,
        exit_tx: &tokio::sync::mpsc::Sender<()>,
    ) {
        use futures::StreamExt; // Import StreamExt for next()
        use futures::SinkExt;   // Import SinkExt for send()

        let mut interval = tokio::time::interval(Duration::from_millis(100));
        let exit_tx = exit_tx.clone();

        loop {
            tokio::select! {
                _ = shutdown_rx.recv() => {
                    info!("收到停止信号，关闭 WebSocket");
                    let _ = ws.close(None).await;
                    let _ = exit_tx.send(()).await;
                    break;
                }
                _ = interval.tick() => {
                    // 处理 WebSocket 消息
                    match tokio::time::timeout(read_timeout, ws.next()).await {
                        Ok(Some(Ok(msg))) => {
                            if let tokio_tungstenite::tungstenite::Message::Text(text) = msg {
                                if let Err(e) = Self::process_message(
                                    &text,
                                    ws,
                                    processed_messages,
                                    event_handler,
                                    client,
                                ).await {
                                    error!(error = %e, "处理消息失败");
                                }
                            }
                        }
                        Ok(None) => {
                            warn!("WebSocket 连接关闭");
                            let _ = exit_tx.send(()).await;
                            break;
                        }
                        Ok(Some(Err(e))) => {
                            error!(error = %e, "WebSocket 读取错误");
                            let _ = exit_tx.send(()).await;
                            break;
                        }
                        Err(_) => {
                            // 超时，继续循环
                        }
                    }
                }
                _ = heartbeat_interval.tick() => {
                    // 发送心跳
                    let ping_msg = serde_json::json!({
                        "type": "ping"
                    });
                    if let Err(e) = ws.send(tokio_tungstenite::tungstenite::Message::Text(ping_msg.to_string())).await {
                        error!(error = %e, "发送心跳失败");
                        let _ = exit_tx.send(()).await;
                        break;
                    }
                }
            }
        }
    }

    /// 处理 WebSocket 消息
    async fn process_message(
        text: &str,
        ws: &mut tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>
        >,
        processed_messages: &Arc<dashmap::DashMap<String, Instant>>,
        event_handler: &MessageEventHandler,
        client: &FeishuClient,
    ) -> Result<(), crate::infra::error::Error> {
        use futures::SinkExt; // Import SinkExt for send()

        debug!(message = %text, "收到 WebSocket 消息");

        // 解析消息
        let msg: WsMessage = serde_json::from_str(text)
            .map_err(|e| crate::infra::error::Error::Serialization(e.to_string()))?;

        match msg {
            WsMessage::Receive { id, content } => {
                // 检查是否是消息事件
                if let Some(event_type) = content.get("type") {
                    if event_type == "im.message.receive_v1" {
                        Self::handle_message_event(
                            &id,
                            content,
                            processed_messages,
                            event_handler,
                            client,
                        ).await;
                    }
                }

                // 发送确认
                let ack = serde_json::json!({
                    "type": "send_ack",
                    "id": id,
                    "code": 0,
                    "msg": "ok"
                });
                let _ = ws.send(tokio_tungstenite::tungstenite::Message::Text(ack.to_string())).await;
            }
            WsMessage::SendAck { id, code, msg } => {
                if code != 0 {
                    error!(id = %id, code = code, msg = %msg, "发送消息失败");
                }
            }
            WsMessage::Error { id, code, msg } => {
                error!(id = %id, code = code, msg = %msg, "WebSocket 错误");
            }
            _ => {}
        }

        Ok(())
    }

    /// 处理消息事件
    async fn handle_message_event(
        event_id: &str,
        event_data: serde_json::Value,
        processed_messages: &Arc<dashmap::DashMap<String, Instant>>,
        event_handler: &MessageEventHandler,
        client: &FeishuClient,
    ) {
        let now = Instant::now();

        // 清理过期条目
        processed_messages.retain(|_, &mut v| now.duration_since(v) < Duration::from_secs(300));

        if processed_messages.contains_key(event_id) {
            return;
        }

        processed_messages.insert(event_id.to_string(), now);

        // 构建事件请求
        let event_request = FeishuEventRequest {
            event_type: "im.message.receive_v1".to_string(),
            event_id: event_id.to_string(),
            created_at: chrono::Utc::now().timestamp(),
            event: event_data,
        };

        match event_handler.handle(&event_request).await {
            Ok(result) => {
                debug!(message_id = %result.message.id, "消息处理成功");
            }
            Err(e) => {
                error!(event_id = %event_id, error = %e, "消息处理失败");
            }
        }
    }
}
