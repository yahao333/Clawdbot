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
    /// 心跳 ping
    #[serde(rename = "ping")]
    Ping,
    /// 心跳响应 pang
    #[serde(rename = "pang")]
    Pang,
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

                            // 启动心跳和读取任务
                            let heartbeat_interval = tokio::time::interval(Duration::from_secs(ws_config.heartbeat_interval_secs));
                            let read_timeout = Duration::from_secs(ws_config.read_timeout_secs);
                            let processed_messages = processed_messages.clone();
                            let event_handler = event_handler.clone();
                            let handler_context = handler_context.clone();
                            let message_handler = message_handler.clone();
                            let client = client.clone();
                            let shutdown_tx_for_tasks = shutdown_tx.clone();
                            let processed_messages_for_poll = processed_messages.clone();
                            let event_handler_for_poll = event_handler.clone();
                            let handler_context_for_poll = handler_context.clone();
                            let message_handler_for_poll = message_handler.clone();
                            let client_for_poll = client.clone();

                            let (exit_tx, mut exit_rx) = tokio::sync::mpsc::channel(1);

                            // 创建内部停止接收器
                            let mut inner_shutdown = shutdown_tx_for_tasks.subscribe();

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

                            // 轮询任务：备用方案，当 WebSocket 没有推送消息时使用
                            let poll_task = tokio::spawn(async move {
                                let mut poll_interval = tokio::time::interval(Duration::from_secs(5));
                                let mut inner_poll_shutdown = shutdown_tx_for_tasks.subscribe();

                                loop {
                                    tokio::select! {
                                        _ = poll_interval.tick() => {
                                            match Self::poll_messages(
                                                &client_for_poll,
                                                &processed_messages_for_poll,
                                                &event_handler_for_poll,
                                                &handler_context_for_poll,
                                                message_handler_for_poll.as_ref(),
                                            ).await {
                                                Ok(count) => {
                                                    if count > 0 {
                                                        info!(count = count, "轮询获取到新消息");
                                                    }
                                                }
                                                Err(e) => {
                                                    debug!(error = %e, "轮询消息失败");
                                                }
                                            }
                                        }
                                        _ = inner_poll_shutdown.recv() => {
                                            info!("轮询任务收到停止信号");
                                            break;
                                        }
                                    }
                                }
                            });

                            // 等待退出信号
                            let _ = exit_rx.recv().await;

                            // 停止读取任务
                            read_task.abort();
                            // 停止轮询任务
                            poll_task.abort();
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
                                    &*handler_context,
                                    &*message_handler,
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
        handler_context: &HandlerContext,
        message_handler: &dyn MessageHandler,
        client: &FeishuClient,
    ) -> Result<(), crate::infra::error::Error> {
        use futures::SinkExt; // Import SinkExt for send()

        // 详细调试日志：显示原始消息
        info!(raw_message = %text, "收到 WebSocket 消息");

        // 解析消息
        let msg: WsMessage = match serde_json::from_str(text) {
            Ok(m) => m,
            Err(e) => {
                error!(error = %e, raw = %text, "解析 WebSocket 消息失败");
                return Err(crate::infra::error::Error::Serialization(e.to_string()));
            }
        };

        // 打印解析后的消息类型
        info!(msg_type = ?msg, "消息解析完成");

        match msg {
            WsMessage::Receive { id, content } => {
                info!(msg_id = %id, "收到 receive 类型消息");
                info!(content = %content, "消息内容");

                // 检查是否是消息事件
                if let Some(event_type) = content.get("type") {
                    info!(event_type = %event_type, "事件类型");

                    // 打印完整的 content 结构
                    info!(full_content = %serde_json::to_string_pretty(&content).unwrap_or_default(), "完整事件数据");

                    if event_type == "im.message.receive_v1" {
                        Self::handle_message_event(
                            &id,
                            content,
                            processed_messages,
                            event_handler,
                            handler_context,
                            message_handler,
                            client,
                        ).await;
                    } else {
                        info!(event_type = %event_type, "跳过非消息事件");
                    }
                } else {
                    warn!(content = %content, "消息中没有 type 字段");
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
            WsMessage::Ping => {
                info!("收到心跳 ping");
                // 响应 pang
                let pong = serde_json::json!({"type": "pang"});
                let _ = ws.send(tokio_tungstenite::tungstenite::Message::Text(pong.to_string())).await;
            }
            WsMessage::Send { id, content } => {
                info!(msg_id = %id, content = %content, "收到 Send 消息");
            }
            WsMessage::Pang => {
                info!("收到心跳响应 pang");
            }
            WsMessage::Unknown => {
                warn!("收到未知类型的 WebSocket 消息");
            }
        }

        Ok(())
    }

    /// 轮询获取消息（备用方案）
    async fn poll_messages(
        client: &FeishuClient,
        processed_messages: &Arc<dashmap::DashMap<String, Instant>>,
        event_handler: &MessageEventHandler,
        handler_context: &HandlerContext,
        message_handler: &dyn MessageHandler,
    ) -> Result<u32, crate::infra::error::Error> {
        // 注意：飞书 API (GET /im/v1/messages) 要求必须提供 container_id (chat_id)。
        // 由于我们无法获取全局所有会话的 ID，且 WebSocket 是主要的接收方式，
        // 这里暂时禁用轮询功能，避免 API 报错 "field validation failed"。
        // 如果需要启用轮询，必须针对特定的 chat_id 进行。
        
        // let response = client.get_messages("chat", "SOME_CHAT_ID", 20).await?;
        
        // 为避免 unused 警告，使用下划线
        let _ = client;
        let _ = processed_messages;
        let _ = event_handler;
        let _ = handler_context;
        let _ = message_handler;

        Ok(0)

        /* 
        // 原始代码保留供参考（但在缺少 chat_id 时无法工作）
        use super::handlers::FeishuMessageItem;

        let response = client.get_messages(20).await?;

        let mut new_count = 0;
        if let Some(data) = response.data {
            for item in data.items {
                // ... (原有逻辑)
            }
        }
        Ok(new_count)
        */
    }

    /// 处理消息事件
    async fn handle_message_event(
        event_id: &str,
        event_data: serde_json::Value,
        processed_messages: &Arc<dashmap::DashMap<String, Instant>>,
        event_handler: &MessageEventHandler,
        handler_context: &HandlerContext,
        message_handler: &dyn MessageHandler,
        client: &FeishuClient,
    ) {
        let now = Instant::now();

        // 清理过期条目
        processed_messages.retain(|_, &mut v| now.duration_since(v) < Duration::from_secs(300));

        if processed_messages.contains_key(event_id) {
            debug!(event_id = %event_id, "消息已处理过，跳过");
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
                // 先保存消息 ID，后面还会用到
                let message_id = result.message.id.clone();
                let chat_id = result.message.target.id.clone();

                info!(
                    message_id = %message_id,
                    chat_id = %chat_id,
                    sender_id = %result.message.sender.id,
                    "收到飞书消息，准备处理"
                );

                // 调用消息处理器处理消息
                match message_handler.handle(handler_context, result.message).await {
                    Ok(_) => {
                        info!(message_id = %message_id, "消息处理完成");
                    }
                    Err(e) => {
                        error!(message_id = %message_id, error = %e, "消息处理器执行失败");
                    }
                }

                // 如果需要已读回执，发送已读标记
                if result.need_read_receipt {
                    // TODO: 发送已读回执
                    debug!(message_id = %message_id, "需要发送已读回执");
                }
            }
            Err(e) => {
                error!(event_id = %event_id, error = %e, "消息事件处理失败");
            }
        }
    }
}
