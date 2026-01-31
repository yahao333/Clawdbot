//! Web 管理界面模块
//!
//! 提供 Web 管理界面和 API 接口。

use axum::{
    routing::{get, post, put},
    Router,
    extract::{State, Query},
    response::{Html, IntoResponse},
    Json,
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
};
use std::sync::Arc;
use std::time::SystemTime;
use tokio::sync::{RwLock, broadcast, Mutex};
use tracing::{info, error, debug};
use serde::{Serialize, Deserialize};
use serde_json::{Value as JsonValue, json};
use std::collections::HashMap;
use base64::{Engine as _, engine::general_purpose::STANDARD};

/// Web 认证配置
#[derive(Debug, Clone)]
pub struct AuthConfig {
    /// 是否启用认证
    pub enabled: bool,
    /// 用户名
    pub username: String,
    /// 密码（bcrypt 或 plaintext）
    pub password: String,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            username: String::new(),
            password: String::new(),
        }
    }
}

// ==================== WebSocket 消息类型 ====================

/// WebSocket 消息类型
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", content = "data")]
pub enum WebSocketMessage {
    /// 统计更新
    #[serde(rename = "stats_update")]
    StatsUpdate(MessageStats),
    /// 服务状态变更
    #[serde(rename = "status_change")]
    StatusChange {
        status: String,
        message: Option<String>,
    },
    /// 新消息
    #[serde(rename = "new_message")]
    NewMessage(Conversation),
    /// 告警通知
    #[serde(rename = "alert")]
    Alert {
        level: String,
        title: String,
        message: String,
        timestamp: String,
    },
    /// 渠道状态变更
    #[serde(rename = "channel_status")]
    ChannelStatus {
        channel: String,
        connected: bool,
        message: Option<String>,
    },
    /// 心跳消息
    #[serde(rename = "ping")]
    Ping,
    /// Pong 响应
    #[serde(rename = "pong")]
    Pong,
}

impl WebSocketMessage {
    /// 将消息转换为 JSON 字符串
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}

/// WebSocket 连接管理器
#[derive(Clone)]
pub struct WebSocketManager {
    /// 广播 sender，用于向所有连接的客户端发送消息
    tx: broadcast::Sender<WebSocketMessage>,
    /// 当前连接的客户端数量
    client_count: Arc<RwLock<u32>>,
}

impl WebSocketManager {
    /// 创建新的 WebSocket 管理器
    pub fn new() -> Self {
        // 创建一个广播 channel，容量为 100
        let (tx, _) = broadcast::channel(100);
        Self {
            tx,
            client_count: Arc::new(RwLock::new(0)),
        }
    }

    /// 获取广播 sender
    pub fn sender(&self) -> broadcast::Sender<WebSocketMessage> {
        self.tx.clone()
    }

    /// 增加客户端计数
    pub async fn add_client(&self) {
        let mut count = self.client_count.write().await;
        *count += 1;
        tracing::info!(client_count = *count, "新的 WebSocket 客户端已连接");
    }

    /// 减少客户端计数
    pub async fn remove_client(&self) {
        let mut count = self.client_count.write().await;
        *count = count.saturating_sub(1);
        tracing::info!(client_count = *count, "WebSocket 客户端已断开");
    }

    /// 获取当前客户端数量
    pub async fn get_client_count(&self) -> u32 {
        *self.client_count.read().await
    }

    /// 广播消息到所有客户端
    pub fn broadcast(&self, message: WebSocketMessage) {
        // 忽略发送失败（客户端可能已断开）
        let _ = self.tx.send(message);
    }

    /// 发送统计更新
    pub fn send_stats_update(&self, stats: MessageStats) {
        self.broadcast(WebSocketMessage::StatsUpdate(stats));
    }

    /// 发送服务状态变更
    pub fn send_status_change(&self, status: ServiceStatus) {
        let (status_str, message) = match &status {
            ServiceStatus::Running => ("running".to_string(), None),
            ServiceStatus::Initializing => ("initializing".to_string(), None),
            ServiceStatus::Stopping => ("stopping".to_string(), None),
            ServiceStatus::Stopped => ("stopped".to_string(), None),
            ServiceStatus::Error(msg) => ("error".to_string(), Some(msg.clone())),
        };
        self.broadcast(WebSocketMessage::StatusChange {
            status: status_str,
            message,
        });
    }

    /// 发送告警
    pub fn send_alert(&self, level: &str, title: &str, message: &str) {
        let timestamp = chrono::Utc::now().to_rfc3339();
        self.broadcast(WebSocketMessage::Alert {
            level: level.to_string(),
            title: title.to_string(),
            message: message.to_string(),
            timestamp,
        });
    }
}

/// Web 服务器状态
#[derive(Clone)]
pub struct WebState {
    /// 唯一标识符（用于调试）
    pub id: String,
    /// 消息统计
    pub message_stats: Arc<RwLock<MessageStats>>,
    /// 对话历史（内存中）
    pub conversation_history: Arc<RwLock<Vec<Conversation>>>,
    /// 配置
    #[allow(dead_code)]
    pub config: Arc<tokio::sync::Mutex<Option<super::infra::config::Config>>>,
    /// 审计服务（可选）
    pub audit_service: Arc<RwLock<Option<super::security::AuditService>>>,
    /// 服务状态（用于健康检查）
    pub service_status: Arc<RwLock<ServiceStatus>>,
    /// AI 引擎状态
    pub ai_engine_status: Arc<RwLock<AIEngineStatus>>,
    /// 渠道连接状态
    pub channel_status: Arc<RwLock<HashMap<String, ChannelConnectionStatus>>>,
    /// 认证配置
    pub auth_config: Arc<RwLock<AuthConfig>>,
    /// WebSocket 管理器
    pub ws_manager: Arc<WebSocketManager>,
}

impl WebState {
    /// 创建新的 WebState（不启动 Web 服务器时使用）
    pub fn new() -> Self {
        let id = format!("{:x}", std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis());
        tracing::info!(id = %id, "创建新的 WebState");
        Self {
            id,
            message_stats: Arc::new(RwLock::new(MessageStats::default())),
            conversation_history: Arc::new(RwLock::new(Vec::new())),
            config: Arc::new(tokio::sync::Mutex::new(None)),
            audit_service: Arc::new(RwLock::new(None)),
            service_status: Arc::new(RwLock::new(ServiceStatus::Running)),
            ai_engine_status: Arc::new(RwLock::new(AIEngineStatus::default())),
            channel_status: Arc::new(RwLock::new(HashMap::new())),
            auth_config: Arc::new(RwLock::new(AuthConfig::default())),
            ws_manager: Arc::new(WebSocketManager::new()),
        }
    }

    /// 更新渠道连接状态
    pub async fn update_channel_status(&self, channel: &str, status: ChannelConnectionStatus) {
        tracing::info!(id = %self.id, channel = channel, connected = status.connected, "开始更新渠道状态");
        let mut channel_status = self.channel_status.write().await;
        channel_status.insert(channel.to_string(), status.clone());
        tracing::info!(id = %self.id, channel_status_count = channel_status.len(), "渠道状态已更新");
        // 广播状态更新
        self.ws_manager.broadcast(WebSocketMessage::ChannelStatus {
            channel: channel.to_string(),
            connected: status.connected,
            message: status.last_error.or(status.last_connected),
        });
    }
}

// ==================== 类型定义 ====================

/// 服务状态枚举
/// 用于健康检查和就绪检查
#[derive(Debug, Clone, Serialize, Default)]
pub enum ServiceStatus {
    /// 服务正在初始化
    #[default]
    Initializing,
    /// 服务正在运行
    Running,
    /// 服务正在停止
    Stopping,
    /// 服务已停止
    Stopped,
    /// 服务发生错误
    Error(String),
}

/// AI 引擎状态
#[derive(Debug, Default, Clone, Serialize)]
pub struct AIEngineStatus {
    pub is_running: bool,
    pub active_providers: Vec<String>,
    pub default_provider: String,
    pub last_error: Option<String>,
}

/// 渠道连接状态
#[derive(Debug, Default, Clone, Serialize)]
pub struct ChannelConnectionStatus {
    pub connected: bool,
    pub last_connected: Option<String>,
    pub last_error: Option<String>,
    pub message_count: u64,
}

/// 消息统计 - 增强版
/// 包含更多维度的监控指标
#[derive(Debug, Default, Clone, Serialize)]
pub struct MessageStats {
    // 消息计数
    pub total_messages: u64,          // 总消息数
    pub today_messages: u64,          // 今日消息数
    pub successful_messages: u64,     // 成功处理的消息数
    pub failed_messages: u64,         // 处理失败的消息数

    // Token 统计
    pub total_tokens: u64,            // 总 Token 数
    pub today_tokens: u64,            // 今日 Token 数
    pub total_input_tokens: u64,      // 总输入 Token
    pub total_output_tokens: u64,     // 总输出 Token

    // 渠道统计
    pub feishu_messages: u64,         // 飞书消息数
    pub telegram_messages: u64,       // Telegram 消息数
    pub discord_messages: u64,        // Discord 消息数

    // 性能统计
    pub avg_response_time_ms: f64,    // 平均响应时间（毫秒）
    pub max_response_time_ms: u64,    // 最大响应时间（毫秒）
    pub min_response_time_ms: u64,    // 最小响应时间（毫秒）

    // AI 统计
    pub ai_requests_total: u64,       // AI 请求总数
    pub ai_requests_success: u64,     // AI 请求成功数
    pub ai_requests_failed: u64,      // AI 请求失败数

    // 会话统计
    pub active_sessions: u64,         // 当前活跃会话数
    pub total_sessions: u64,          // 历史会话总数
}

/// 对话记录
#[derive(Debug, Clone, Serialize)]
pub struct Conversation {
    pub id: String,
    pub channel: String,
    pub user_id: String,
    pub user_name: Option<String>,
    pub message: String,
    pub response: String,
    pub tokens: u32,
    #[serde(skip)] // 暂时跳过序列化时间戳
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// 发送消息请求
#[derive(Debug, Clone, Deserialize)]
pub struct SendMessageRequest {
    pub channel: String,
    pub target_id: String,
    pub message: String,
}

/// 发送消息响应
#[derive(Debug, Clone, Serialize)]
pub struct SendMessageResponse {
    pub success: bool,
    pub message_id: Option<String>,
    pub response: Option<String>,
    pub error: Option<String>,
}

/// 配置更新请求
#[derive(Debug, Clone, Deserialize)]
pub struct ConfigUpdateRequest {
    pub key: String,
    pub value: JsonValue,
}

// ==================== 认证中间件 ====================

/// 认证中间件状态
/// 用于在中间件中传递认证配置
#[derive(Clone)]
struct AuthMiddlewareState {
    auth_config: Arc<RwLock<AuthConfig>>,
}

impl AuthMiddlewareState {
    fn new(auth_config: Arc<RwLock<AuthConfig>>) -> Self {
        Self { auth_config }
    }
}

/// Basic Auth 认证中间件
/// 检查请求的 Authorization 头，验证用户名和密码
async fn auth_middleware(
    State(auth_state): State<AuthMiddlewareState>,
    request: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> Result<axum::response::Response, std::convert::Infallible> {
    // 获取认证配置
    let auth_config = auth_state.auth_config.read().await;

    // 如果认证未启用，直接放行
    if !auth_config.enabled {
        debug!("认证已跳过（未启用）");
        return Ok(next.run(request).await);
    }

    // 记录调试信息
    debug!(enabled = auth_config.enabled, username = auth_config.username, "认证检查");

    // 检查 Authorization 头
    let auth_header = request.headers()
        .get(axum::http::header::AUTHORIZATION)
        .map(|v| v.to_str().unwrap_or(""));

    match auth_header {
        Some("") | None => {
            // 没有 Authorization 头，返回 401
            debug!("认证失败：缺少 Authorization 头");
            return create_401_response();
        }
        Some(auth) if auth.starts_with("Basic ") => {
            // 解析 Basic Auth
            let credentials = auth.trim_start_matches("Basic ");
            match STANDARD.decode(credentials) {
                Ok(decoded) => {
                    let credentials_str = String::from_utf8_lossy(&decoded);
                    debug!("解码后的凭据: {}", credentials_str);

                    let parts: Vec<&str> = credentials_str.split(':').collect();
                    if parts.len() >= 2 {
                        let username = parts[0];
                        let password = parts[1..].join(":");

                        debug!(input_username = username, expected_username = auth_config.username);
                        debug!(password_match = (password == auth_config.password));

                        // 验证凭据
                        if username == auth_config.username && password == auth_config.password {
                            debug!("认证成功");
                            return Ok(next.run(request).await);
                        } else {
                            debug!("认证失败：凭据错误");
                            return create_401_response();
                        }
                    } else {
                        debug!("认证失败：凭据格式错误");
                        return create_401_response();
                    }
                }
                Err(e) => {
                    debug!(error = ?e, "认证失败：Base64 解码失败");
                    return create_401_response();
                }
            }
        }
        Some(auth) => {
            debug!(auth_type = %auth, "认证失败：不支持的认证方式");
            return create_401_response();
        }
    }
}

/// 创建 401 Unauthorized 响应
fn create_401_response() -> Result<axum::response::Response, std::convert::Infallible> {
    let response = axum::response::Response::builder()
        .status(axum::http::StatusCode::UNAUTHORIZED)
        .header(axum::http::header::WWW_AUTHENTICATE, "Basic realm=\"Clawdbot\"")
        .body(axum::body::Body::empty())
        .unwrap();
    Ok(response)
}

// ==================== WebSocket 处理器 ====================

/// WebSocket 连接处理器
/// 处理 WebSocket 连接，接收和发送消息
async fn ws_handler(
    State(state): State<WebState>,
    ws: WebSocketUpgrade,
) -> axum::response::Response {
    let ws_manager = state.ws_manager.clone();

    ws.on_upgrade(move |socket| {
        handle_socket(ws_manager, socket)
    })
}

/// 处理 WebSocket 连接的核心逻辑
async fn handle_socket(ws_manager: Arc<WebSocketManager>, ws: WebSocket) {
    use futures_util::{StreamExt, SinkExt};

    // 增加客户端计数
    ws_manager.add_client().await;

    // 获取广播 receiver
    let mut rx = ws_manager.sender().subscribe();

    let (sink, stream) = ws.split();
    let sink = Arc::new(Mutex::new(sink));
    let stream = std::pin::Pin::new(Box::new(stream));

    // 任务1：接收客户端消息
    let receive_task = async {
        let mut stream = stream;
        let sink = sink.clone();
        while let Some(result) = stream.next().await {
            match result {
                Ok(Message::Text(text)) => {
                    tracing::debug!(message = %text, "收到 WebSocket 消息");
                    handle_ws_message(&text, &ws_manager).await;
                }
                Ok(Message::Binary(data)) => {
                    tracing::debug!(length = data.len(), "收到二进制消息");
                }
                Ok(Message::Ping(_)) => {
                    let mut guard = sink.lock().await;
                    let _ = guard.send(Message::Pong(vec![])).await;
                }
                Ok(Message::Pong(_)) => {}
                Ok(Message::Close(_)) => {
                    break;
                }
                Err(e) => {
                    tracing::debug!(error = %e, "WebSocket 接收错误");
                    break;
                }
            }
        }
    };

    // 任务2：广播消息到客户端
    let broadcast_task = async {
        let mut rx = rx;
        let sink = sink.clone();
        while let Ok(message) = rx.recv().await {
            let json = message.to_json();
            let mut guard = sink.lock().await;
            if guard.send(Message::Text(json)).await.is_err() {
                break;
            }
        }
    };

    // 并发执行两个任务
    tokio::select! {
        _ = receive_task => {},
        _ = broadcast_task => {},
    }

    // 减少客户端计数
    ws_manager.remove_client().await;
}

/// 处理 WebSocket 客户端消息
async fn handle_ws_message(text: &str, _ws_manager: &WebSocketManager) {
    // 解析消息类型
    if let Ok(message) = serde_json::from_str::<serde_json::Value>(text) {
        let msg_type = message.get("type").and_then(|v| v.as_str()).unwrap_or("");

        match msg_type {
            "ping" => {
                // 心跳消息，客户端要求响应 pong
                // 实际响应在 broadcast_task 中处理
                tracing::debug!("收到心跳消息");
            }
            "subscribe" => {
                // 订阅特定事件
                tracing::debug!(message = %text, "订阅请求");
            }
            "unsubscribe" => {
                // 取消订阅
                tracing::debug!(message = %text, "取消订阅请求");
            }
            _ => {
                tracing::debug!(msg_type = msg_type, "未知的 WebSocket 消息类型");
            }
        }
    }
}

/// 获取 WebSocket 连接状态的处理器
async fn ws_status_handler(State(state): State<WebState>) -> Json<JsonValue> {
    let client_count = state.ws_manager.get_client_count().await;
    Json(json!({
        "connected": true,
        "client_count": client_count,
        "url": "ws://localhost/ws"
    }))
}

// ==================== 认证 API ====================

/// 登出处理器
/// 清除浏览器缓存的 Basic Auth 凭据
/// 通过返回 401 并设置认证头，浏览器会弹出新的认证框
async fn logout_handler() -> Result<axum::response::Response, std::convert::Infallible> {
    info!("用户请求登出");

    // 返回 401 并清除 WWW-Authenticate 头
    let response = axum::response::Response::builder()
        .status(axum::http::StatusCode::UNAUTHORIZED)
        .header(axum::http::header::WWW_AUTHENTICATE, "Basic realm=\"Clawdbot\", charset=\"UTF-8\"")
        .body(axum::body::Body::empty())
        .unwrap();
    Ok(response)
}

/// 获取当前认证状态的处理器
async fn auth_status_handler(State(state): State<WebState>) -> Json<JsonValue> {
    let auth_config = state.auth_config.read().await;
    Json(json!({
        "enabled": auth_config.enabled,
        "username": if auth_config.enabled { &auth_config.username } else { "" },
    }))
}

// ==================== 路由处理器 ====================

// 首页
async fn index_handler() -> Html<&'static str> {
    Html(HTML_INDEX)
}

// 调试监控页面
async fn debug_handler() -> Html<&'static str> {
    Html(HTML_DEBUG)
}

// 配置管理页面
async fn config_handler() -> Html<&'static str> {
    Html(HTML_CONFIG)
}

// 运营数据页面
async fn operations_handler() -> Html<&'static str> {
    Html(HTML_OPERATIONS)
}

// ==================== 健康检查端点 ====================

/// 健康检查端点（Liveness Probe）
/// 用于 Kubernetes 检查容器是否存活
/// 返回 200 表示服务正常运行
async fn health_handler(State(state): State<WebState>) -> (axum::http::StatusCode, &'static str) {
    let status = state.service_status.read().await;
    match &*status {
        ServiceStatus::Running => {
            (axum::http::StatusCode::OK, "OK")
        }
        ServiceStatus::Initializing => {
            (axum::http::StatusCode::OK, "Initializing")
        }
        ServiceStatus::Stopping => {
            (axum::http::StatusCode::SERVICE_UNAVAILABLE, "Stopping")
        }
        ServiceStatus::Stopped => {
            (axum::http::StatusCode::SERVICE_UNAVAILABLE, "Stopped")
        }
        ServiceStatus::Error(msg) => {
            tracing::warn!(error = %msg, "健康检查失败");
            (axum::http::StatusCode::SERVICE_UNAVAILABLE, "Error")
        }
    }
}

/// 就绪检查端点（Readiness Probe）
/// 用于 Kubernetes 检查服务是否准备好接收流量
/// 检查服务状态、AI 引擎、渠道连接
async fn ready_handler(State(state): State<WebState>) -> (axum::http::StatusCode, &'static str) {
    let status = state.service_status.read().await;

    // 检查服务状态
    match &*status {
        ServiceStatus::Running => {}
        ServiceStatus::Initializing => {
            return (axum::http::StatusCode::SERVICE_UNAVAILABLE, "Not Ready: Initializing");
        }
        ServiceStatus::Stopping => {
            return (axum::http::StatusCode::SERVICE_UNAVAILABLE, "Not Ready: Stopping");
        }
        ServiceStatus::Stopped => {
            return (axum::http::StatusCode::SERVICE_UNAVAILABLE, "Not Ready: Stopped");
        }
        ServiceStatus::Error(msg) => {
            tracing::warn!(error = %msg, "就绪检查失败");
            return (axum::http::StatusCode::SERVICE_UNAVAILABLE, "Not Ready: Error");
        }
    }

    // 检查 AI 引擎状态
    let ai_status = state.ai_engine_status.read().await;
    if !ai_status.is_running {
        return (axum::http::StatusCode::SERVICE_UNAVAILABLE, "Not Ready: AI engine not running");
    }

    // 检查渠道连接状态
    let channel_status = state.channel_status.read().await;
    for (name, status) in channel_status.iter() {
        if !status.connected {
            tracing::debug!(channel = %name, "渠道未连接");
        }
    }

    // 所有检查通过
    (axum::http::StatusCode::OK, "Ready")
}

/// Prometheus 指标端点
/// 返回 Prometheus 格式的监控指标
async fn metrics_handler(State(state): State<WebState>) -> String {
    let stats = state.service_status.read().await;
    let message_stats = state.message_stats.read().await;
    let ai_stats = state.ai_engine_status.read().await;
    let channel_stats = state.channel_status.read().await;

    let mut output = String::new();

    // 通用指标
    output.push_str("# HELP clawdbot_up 服务是否运行\n");
    output.push_str("# TYPE clawdbot_up gauge\n");
    let up = matches!(*stats, ServiceStatus::Running);
    output.push_str(&format!("clawdbot_up {}\n", if up { 1 } else { 0 }));

    // 消息统计
    output.push_str("\n# HELP clawdbot_messages_total 总消息数\n");
    output.push_str("# TYPE clawdbot_messages_total counter\n");
    output.push_str(&format!("clawdbot_messages_total {}\n", message_stats.total_messages));

    output.push_str("\n# HELP clawdbot_messages_today 今日消息数\n");
    output.push_str("# TYPE clawdbot_messages_today gauge\n");
    output.push_str(&format!("clawdbot_messages_today {}\n", message_stats.today_messages));

    output.push_str("\n# HELP clawdbot_messages_success 成功处理的消息数\n");
    output.push_str("# TYPE clawdbot_messages_success counter\n");
    output.push_str(&format!("clawdbot_messages_success {}\n", message_stats.successful_messages));

    output.push_str("\n# HELP clawdbot_messages_failed 失败的消息数\n");
    output.push_str("# TYPE clawdbot_messages_failed counter\n");
    output.push_str(&format!("clawdbot_messages_failed {}\n", message_stats.failed_messages));

    // Token 统计
    output.push_str("\n# HELP clawdbot_tokens_total 总 Token 数\n");
    output.push_str("# TYPE clawdbot_tokens_total counter\n");
    output.push_str(&format!("clawdbot_tokens_total {}\n", message_stats.total_tokens));

    output.push_str("\n# HELP clawdbot_tokens_today 今日 Token 数\n");
    output.push_str("# TYPE clawdbot_tokens_today gauge\n");
    output.push_str(&format!("clawdbot_tokens_today {}\n", message_stats.today_tokens));

    output.push_str("\n# HELP clawdbot_tokens_input 输入 Token 数\n");
    output.push_str("# TYPE clawdbot_tokens_input counter\n");
    output.push_str(&format!("clawdbot_tokens_input {}\n", message_stats.total_input_tokens));

    output.push_str("\n# HELP clawdbot_tokens_output 输出 Token 数\n");
    output.push_str("# TYPE clawdbot_tokens_output counter\n");
    output.push_str(&format!("clawdbot_tokens_output {}\n", message_stats.total_output_tokens));

    // 渠道统计
    output.push_str("\n# HELP clawdbot_channel_messages 渠道消息数\n");
    output.push_str("# TYPE clawdbot_channel_messages counter\n");
    output.push_str(&format!("clawdbot_channel_messages{{channel=\"feishu\"}} {}\n", message_stats.feishu_messages));
    output.push_str(&format!("clawdbot_channel_messages{{channel=\"telegram\"}} {}\n", message_stats.telegram_messages));
    output.push_str(&format!("clawdbot_channel_messages{{channel=\"discord\"}} {}\n", message_stats.discord_messages));

    // 性能统计
    output.push_str("\n# HELP clawdbot_response_time_avg 平均响应时间（毫秒）\n");
    output.push_str("# TYPE clawdbot_response_time_avg gauge\n");
    output.push_str(&format!("clawdbot_response_time_avg {}\n", message_stats.avg_response_time_ms));

    output.push_str("\n# HELP clawdbot_response_time_max 最大响应时间（毫秒）\n");
    output.push_str("# TYPE clawdbot_response_time_max gauge\n");
    output.push_str(&format!("clawdbot_response_time_max {}\n", message_stats.max_response_time_ms));

    // AI 统计
    output.push_str("\n# HELP clawdbot_ai_requests_total AI 请求总数\n");
    output.push_str("# TYPE clawdbot_ai_requests_total counter\n");
    output.push_str(&format!("clawdbot_ai_requests_total {}\n", message_stats.ai_requests_total));

    output.push_str("\n# HELP clawdbot_ai_requests_success AI 请求成功数\n");
    output.push_str("# TYPE clawdbot_ai_requests_success counter\n");
    output.push_str(&format!("clawdbot_ai_requests_success {}\n", message_stats.ai_requests_success));

    output.push_str("\n# HELP clawdbot_ai_requests_failed AI 请求失败数\n");
    output.push_str("# TYPE clawdbot_ai_requests_failed counter\n");
    output.push_str(&format!("clawdbot_ai_requests_failed {}\n", message_stats.ai_requests_failed));

    // 会话统计
    output.push_str("\n# HELP clawdbot_active_sessions 当前活跃会话数\n");
    output.push_str("# TYPE clawdbot_active_sessions gauge\n");
    output.push_str(&format!("clawdbot_active_sessions {}\n", message_stats.active_sessions));

    // 渠道连接状态
    output.push_str("\n# HELP clawdbot_channel_connected 渠道连接状态\n");
    output.push_str("# TYPE clawdbot_channel_connected gauge\n");
    for (name, status) in channel_stats.iter() {
        output.push_str(&format!("clawdbot_channel_connected{{channel=\"{}\"}} {}\n",
            name, if status.connected { 1 } else { 0 }));
    }

    // AI 引擎状态
    output.push_str("\n# HELP clawdbot_ai_engine_running AI 引擎运行状态\n");
    output.push_str("# TYPE clawdbot_ai_engine_running gauge\n");
    output.push_str(&format!("clawdbot_ai_engine_running {}\n",
        if ai_stats.is_running { 1 } else { 0 }));

    output
}

/// 获取详细服务状态
async fn api_status_handler(State(state): State<WebState>) -> Json<JsonValue> {
    let service_status = state.service_status.read().await;
    let ai_status = state.ai_engine_status.read().await;
    let channel_status = state.channel_status.read().await;

    // 构建渠道状态的 HashMap
    let mut channels_map = serde_json::Map::new();
    for (name, status) in &*channel_status {
        let mut channel_obj = serde_json::Map::new();
        channel_obj.insert("connected".to_string(), json!(status.connected));
        channel_obj.insert("last_connected".to_string(), json!(status.last_connected));
        channel_obj.insert("last_error".to_string(), json!(status.last_error));
        channel_obj.insert("message_count".to_string(), json!(status.message_count));
        channels_map.insert(name.clone(), json!(channel_obj));
    }

    Json(json!({
        "service": {
            "status": format!("{:?}", *service_status),
        },
        "ai_engine": {
            "is_running": ai_status.is_running,
            "active_providers": ai_status.active_providers,
            "default_provider": ai_status.default_provider,
            "last_error": ai_status.last_error,
        },
        "channels": channels_map,
    }))
}

// 发送测试消息 - 发送到渠道并调用 AI
async fn api_send_message(
    State(state): State<WebState>,
    Json(req): Json<SendMessageRequest>,
) -> Json<SendMessageResponse> {
    info!(channel = %req.channel, target = %req.target_id, message = %req.message, "收到测试消息请求");

    // 根据渠道发送消息
    match req.channel.as_str() {
        "feishu" => {
            // 创建飞书客户端并发送消息
            match send_feishu_message(&req.target_id, &req.message).await {
                Ok(msg_id) => {
                    info!(message_id = %msg_id, "飞书消息发送成功");

                    // 调用 AI 生成响应
                    let ai_response = call_minimax_ai(&req.message, None, None).await;

                    match ai_response {
                        Ok(response) => {
                            // 记录到历史
                            let mut history = state.conversation_history.write().await;
                            history.push(Conversation {
                                id: uuid::Uuid::new_v4().to_string(),
                                channel: req.channel.clone(),
                                user_id: req.target_id.clone(),
                                user_name: None,
                                message: req.message.clone(),
                                response: response.content.clone(),
                                tokens: response.usage.total_tokens,
                                timestamp: chrono::Utc::now(),
                            });

                            // 更新统计
                            let mut stats = state.message_stats.write().await;
                            stats.total_messages += 1;
                            stats.today_messages += 1;
                            stats.total_tokens += response.usage.total_tokens as u64;
                            stats.today_tokens += response.usage.total_tokens as u64;
                            stats.feishu_messages += 1;

                            Json(SendMessageResponse {
                                success: true,
                                message_id: Some(msg_id),
                                response: Some(response.content),
                                error: None,
                            })
                        }
                        Err(e) => {
                            error!(error = %e, "AI 调用失败");
                            Json(SendMessageResponse {
                                success: false,
                                message_id: Some(msg_id),
                                response: None,
                                error: Some(format!("消息已发送，但 AI 调用失败: {}", e)),
                            })
                        }
                    }
                }
                Err(e) => {
                    error!(error = %e, "飞书消息发送失败");
                    Json(SendMessageResponse {
                        success: false,
                        message_id: None,
                        response: None,
                        error: Some(format!("飞书消息发送失败: {}", e)),
                    })
                }
            }
        }
        "telegram" | "discord" => {
            // TODO: 实现 Telegram/Discord 发送
            error!(channel = %req.channel, "渠道发送暂未实现");
            Json(SendMessageResponse {
                success: false,
                message_id: None,
                response: None,
                error: Some(format!("渠道 {} 发送功能暂未实现", req.channel)),
            })
        }
        _ => {
            error!(channel = %req.channel, "不支持的渠道");
            Json(SendMessageResponse {
                success: false,
                message_id: None,
                response: None,
                error: Some(format!("不支持的渠道: {}", req.channel)),
            })
        }
    }
}

/// 发送飞书消息
async fn send_feishu_message(target_id: &str, message: &str) -> Result<String, String> {
    use crate::channels::feishu::send::{FeishuMessageSender, MessageReceiver};
    use crate::channels::feishu::client::FeishuClient;

    // 从环境变量加载飞书配置
    let app_id = std::env::var("FEISHU_APP_ID")
        .map_err(|_| "FEISHU_APP_ID 未设置")?;
    let app_secret = std::env::var("FEISHU_APP_SECRET")
        .map_err(|_| "FEISHU_APP_SECRET 未设置")?;

    info!(target_id = %target_id, "创建飞书客户端并发送消息");

    // 创建凭据和客户端
    let credentials = FeishuClient::new(crate::channels::feishu::client::FeishuCredentials {
        app_id,
        app_secret,
        verification_token: None,
        encrypt_key: None,
    });
    let sender = FeishuMessageSender::from_client(credentials);

    // 确定接收者类型
    let receiver = if target_id.starts_with("ou_") {
        MessageReceiver::OpenId(target_id.to_string())
    } else if target_id.starts_with("oc_") {
        MessageReceiver::ChatId(target_id.to_string())
    } else {
        // 默认当作 Open ID 处理
        MessageReceiver::OpenId(target_id.to_string())
    };

    // 发送文本消息
    let response = sender.send_text(&receiver, message)
        .await
        .map_err(|e| e.to_string())?;

    Ok(response.message_id)
}

/// 大模型测试请求
#[derive(Debug, Deserialize)]
pub struct AiTestRequest {
    pub model: String,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub system_prompt: Option<String>,
    pub message: String,
}

/// 大模型测试响应
#[derive(Debug, Serialize)]
pub struct AiTestResponse {
    pub success: bool,
    pub response: Option<String>,
    pub usage: Option<TokenUsageInfo>,
    pub response_time_ms: Option<u64>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct TokenUsageInfo {
    pub total_tokens: u32,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
}

/// 大模型测试 API
async fn api_ai_test(
    State(state): State<WebState>,
    Json(req): Json<AiTestRequest>,
) -> Json<AiTestResponse> {
    info!(model = %req.model, temperature = ?req.temperature, "收到大模型测试请求");

    let start_time = std::time::Instant::now();

    // 根据选择的模型调用对应的 AI
    let result = match req.model.as_str() {
        "minimax" => {
            call_minimax_ai(&req.message, req.temperature, req.max_tokens).await
        }
        "openai" => {
            call_openai_ai(&req.message, req.temperature, req.max_tokens).await
        }
        "anthropic" => {
            call_anthropic_ai(&req.message, req.temperature, req.max_tokens).await
        }
        _ => Err(format!("不支持的模型: {}", req.model)),
    };

    let response_time_ms = start_time.elapsed().as_millis() as u64;

    match result {
        Ok(response) => {
            // 记录到历史
            let mut history = state.conversation_history.write().await;
            history.push(Conversation {
                id: uuid::Uuid::new_v4().to_string(),
                channel: "debug_ai".to_string(),
                user_id: req.model.clone(),
                user_name: None,
                message: req.message.clone(),
                response: response.content.clone(),
                tokens: response.usage.total_tokens,
                timestamp: chrono::Utc::now(),
            });

            // 更新统计
            let mut stats = state.message_stats.write().await;
            stats.total_messages += 1;
            stats.today_messages += 1;
            stats.total_tokens += response.usage.total_tokens as u64;
            stats.today_tokens += response.usage.total_tokens as u64;

            Json(AiTestResponse {
                success: true,
                response: Some(response.content),
                usage: Some(TokenUsageInfo {
                    total_tokens: response.usage.total_tokens,
                    prompt_tokens: response.usage.prompt_tokens,
                    completion_tokens: response.usage.completion_tokens,
                }),
                response_time_ms: Some(response_time_ms),
                error: None,
            })
        }
        Err(e) => {
            error!(error = %e, "AI 调用失败");
            Json(AiTestResponse {
                success: false,
                response: None,
                usage: None,
                response_time_ms: Some(response_time_ms),
                error: Some(e.to_string()),
            })
        }
    }
}

/// 调用 MiniMax AI
async fn call_minimax_ai(
    message: &str,
    temperature: Option<f32>,
    max_tokens: Option<u32>,
) -> Result<super::ai::provider::ChatResponse, String> {
    use super::ai::provider::AiProvider;

    // 从环境变量加载配置
    let api_key = std::env::var("MINIMAX_API_KEY")
        .map_err(|_| "MINIMAX_API_KEY 未设置")?;
    let group_id = std::env::var("MINIMAX_GROUP_ID")
        .unwrap_or_else(|_| "default".to_string());
    let model = std::env::var("MINIMAX_MODEL")
        .unwrap_or_else(|_| "MiniMax-M2.1".to_string());
    let temp = temperature.unwrap_or_else(|| {
        std::env::var("MINIMAX_TEMPERATURE")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.7)
    });
    let mt = max_tokens.unwrap_or_else(|| {
        std::env::var("MINIMAX_MAX_TOKENS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(4096)
    });

    info!(model = %model, temperature = temp, max_tokens = mt, "使用 MiniMax 模型");

    // 创建 MiniMax 配置
    let config = super::ai::provider::minimax::MiniMaxConfig {
        api_key,
        group_id,
        model: Some(model.clone()),
        temperature: Some(temp),
        max_tokens: Some(mt),
        base_url: None,
    };

    // 创建 Provider
    let provider = super::ai::provider::minimax::MiniMaxProvider::new(config);

    // 构建请求
    let request = super::ai::provider::ChatRequest {
        model: super::ai::provider::ModelConfig {
            provider: "minimax".to_string(),
            model: model.clone(),
            api_key: None,
            base_url: None,
            temperature: Some(temp),
            max_tokens: Some(mt),
            system_prompt: None,
        },
        messages: vec![super::ai::provider::ChatMessage {
            role: super::ai::provider::MessageRole::User,
            content: message.to_string(),
            name: None,
        }],
        tools: vec![],
        stream: false,
    };

    // 调用 AI（通过 trait 对象）
    let provider: &dyn AiProvider = &provider;
    provider.chat(&request)
        .await
        .map_err(|e| e.to_string())
}

/// 调用 OpenAI AI
async fn call_openai_ai(
    message: &str,
    temperature: Option<f32>,
    max_tokens: Option<u32>,
) -> Result<super::ai::provider::ChatResponse, String> {
    use super::ai::provider::AiProvider;

    // 从环境变量加载配置
    let api_key = std::env::var("OPENAI_API_KEY")
        .map_err(|_| "OPENAI_API_KEY 未设置")?;
    let model = std::env::var("OPENAI_MODEL")
        .unwrap_or_else(|_| "gpt-3.5-turbo".to_string());
    let temp = temperature.unwrap_or(0.7);
    let mt = max_tokens.unwrap_or(4096);

    info!(model = %model, temperature = temp, max_tokens = mt, "使用 OpenAI 模型");

    // 创建 OpenAI 配置
    let config = super::ai::provider::openai::OpenAIConfig {
        api_key,
        model: Some(model.clone()),
        temperature: Some(temp),
        max_tokens: Some(mt),
        base_url: None,
        organization_id: None,
    };

    // 创建 Provider
    let provider = super::ai::provider::openai::OpenAIProvider::new(config);

    // 构建请求
    let request = super::ai::provider::ChatRequest {
        model: super::ai::provider::ModelConfig {
            provider: "openai".to_string(),
            model: model.clone(),
            api_key: None,
            base_url: None,
            temperature: Some(temp),
            max_tokens: Some(mt),
            system_prompt: None,
        },
        messages: vec![super::ai::provider::ChatMessage {
            role: super::ai::provider::MessageRole::User,
            content: message.to_string(),
            name: None,
        }],
        tools: vec![],
        stream: false,
    };

    // 调用 AI（通过 trait 对象）
    let provider: &dyn AiProvider = &provider;
    provider.chat(&request)
        .await
        .map_err(|e| e.to_string())
}

/// 调用 Anthropic AI
async fn call_anthropic_ai(
    message: &str,
    temperature: Option<f32>,
    max_tokens: Option<u32>,
) -> Result<super::ai::provider::ChatResponse, String> {
    use super::ai::provider::AiProvider;

    // 从环境变量加载配置
    let api_key = std::env::var("ANTHROPIC_API_KEY")
        .map_err(|_| "ANTHROPIC_API_KEY 未设置")?;
    let model = std::env::var("ANTHROPIC_MODEL")
        .unwrap_or_else(|_| "claude-sonnet-4-20250514".to_string());
    let temp = temperature.unwrap_or(0.7);
    let mt = max_tokens.unwrap_or(4096);

    info!(model = %model, temperature = temp, max_tokens = mt, "使用 Anthropic 模型");

    // 创建 Anthropic 配置
    let config = super::ai::provider::anthropic::AnthropicConfig {
        api_key,
        model: Some(model.clone()),
        temperature: Some(temp),
        max_tokens: Some(mt),
        base_url: None,
        api_version: Some("2023-06-01".to_string()),
    };

    // 创建 Provider
    let provider = super::ai::provider::anthropic::AnthropicProvider::new(config);

    // 构建请求
    let request = super::ai::provider::ChatRequest {
        model: super::ai::provider::ModelConfig {
            provider: "anthropic".to_string(),
            model: model.clone(),
            api_key: None,
            base_url: None,
            temperature: Some(temp),
            max_tokens: Some(mt),
            system_prompt: None,
        },
        messages: vec![super::ai::provider::ChatMessage {
            role: super::ai::provider::MessageRole::User,
            content: message.to_string(),
            name: None,
        }],
        tools: vec![],
        stream: false,
    };

    // 调用 AI（通过 trait 对象）
    let provider: &dyn AiProvider = &provider;
    provider.chat(&request)
        .await
        .map_err(|e| e.to_string())
}

// 获取对话历史
async fn api_conversation_history(State(state): State<WebState>) -> Json<Vec<Conversation>> {
    let history = state.conversation_history.read().await;
    Json(history.clone())
}

// 获取统计信息
async fn api_stats(State(state): State<WebState>) -> Json<MessageStats> {
    let stats = state.message_stats.read().await;
    Json(stats.clone())
}

/// 仪表盘统计数据
#[derive(Debug, Clone, Serialize)]
pub struct DashboardStats {
    /// 服务状态
    pub service_status: String,
    /// 今日消息数
    pub today_messages: u64,
    /// 活跃会话数
    pub active_sessions: u64,
    /// AI 成功率 (%)
    pub ai_success_rate: f64,
    /// 平均响应时间 (ms)
    pub avg_response_time_ms: f64,
    /// 今日 Token 使用量
    pub today_tokens: u64,
    /// 渠道状态列表
    pub channels: Vec<ChannelInfo>,
    /// 最近消息列表
    pub recent_messages: Vec<RecentMessage>,
}

/// 渠道信息
#[derive(Debug, Clone, Serialize)]
pub struct ChannelInfo {
    /// 渠道名称
    pub name: String,
    /// 显示名称
    pub display_name: String,
    /// 是否已连接
    pub connected: bool,
    /// 消息数量
    pub message_count: u64,
    /// 图标
    pub icon: String,
}

/// 最近消息
#[derive(Debug, Clone, Serialize)]
pub struct RecentMessage {
    /// 时间
    pub time: String,
    /// 渠道
    pub channel: String,
    /// 用户
    pub user: String,
    /// 内容摘要
    pub content: String,
}

/// 获取仪表盘统计数据
async fn api_dashboard_stats(State(state): State<WebState>) -> Json<DashboardStats> {
    let service_status = state.service_status.read().await;
    let message_stats = state.message_stats.read().await;
    let channel_status = state.channel_status.read().await;
    let history = state.conversation_history.read().await;

    tracing::info!(id = %state.id, channel_status_len = channel_status.len(), "API 读取渠道状态");
    for (name, status) in &*channel_status {
        tracing::info!(id = %state.id, channel = name, connected = status.connected, "API 读取到渠道状态");
    }

    // 计算 AI 成功率
    let ai_success_rate = if message_stats.ai_requests_total > 0 {
        (message_stats.ai_requests_success as f64 / message_stats.ai_requests_total as f64) * 100.0
    } else {
        100.0
    };

    // 构建渠道状态列表
    let mut channels = Vec::new();
    for (name, status) in &*channel_status {
        let (display_name, icon) = match name.as_str() {
            "feishu" => ("飞书", "🔗"),
            "telegram" => ("Telegram", "📱"),
            "discord" => ("Discord", "💬"),
            _ => (name.as_str(), "📡"),
        };
        channels.push(ChannelInfo {
            name: name.clone(),
            display_name: display_name.to_string(),
            connected: status.connected,
            message_count: status.message_count,
            icon: icon.to_string(),
        });
    }

    // 如果没有渠道状态，添加默认飞书
    if channels.is_empty() {
        tracing::warn!("API 未读取到任何渠道状态，添加默认飞书（未连接）");
        channels.push(ChannelInfo {
            name: "feishu".to_string(),
            display_name: "飞书".to_string(),
            connected: false,
            message_count: 0,
            icon: "🔗".to_string(),
        });
    }

    // 构建最近消息列表（取最近5条）
    let recent_messages: Vec<RecentMessage> = history
        .iter()
        .rev()
        .take(5)
        .map(|conv| {
            let content = if conv.message.len() > 50 {
                &conv.message[..50]
            } else {
                &conv.message
            }.to_string();
            RecentMessage {
                time: conv.timestamp.format("%H:%M").to_string(),
                channel: conv.channel.clone(),
                user: conv.user_name.clone().unwrap_or_else(|| conv.user_id.clone()),
                content,
            }
        })
        .collect();

    let status_str = match &*service_status {
        ServiceStatus::Running => "running",
        ServiceStatus::Initializing => "initializing",
        ServiceStatus::Stopping => "stopping",
        ServiceStatus::Stopped => "stopped",
        ServiceStatus::Error(_) => "error",
    };

    Json(DashboardStats {
        service_status: status_str.to_string(),
        today_messages: message_stats.today_messages,
        active_sessions: message_stats.active_sessions,
        ai_success_rate,
        avg_response_time_ms: message_stats.avg_response_time_ms,
        today_tokens: message_stats.today_tokens,
        channels,
        recent_messages,
    })
}

// 清除历史
async fn api_clear_history(State(state): State<WebState>) -> Json<JsonValue> {
    let mut history = state.conversation_history.write().await;
    history.clear();
    Json(json!({"success": true}))
}

// 获取配置
async fn api_get_config(State(state): State<WebState>) -> Json<JsonValue> {
    let config = state.config.lock().await;
    if let Some(cfg) = &*config {
        Json(json!({
            "ai": cfg.ai,
            "channels": cfg.channels,
        }))
    } else {
        Json(json!({"error": "配置未加载"}))
    }
}

// 更新配置
async fn api_update_config(State(_state): State<WebState>, Json(_req): Json<ConfigUpdateRequest>) -> Json<JsonValue> {
    Json(json!({
        "success": false,
        "message": "配置更新暂未实现"
    }))
}

// 获取 Providers
async fn api_get_providers() -> Json<JsonValue> {
    Json(json!({
        "providers": ["minimax", "openai", "anthropic"],
        "default": "minimax"
    }))
}

// 获取 Channels
async fn api_get_channels() -> Json<JsonValue> {
    Json(json!({
        "channels": ["feishu"],
        "enabled": ["feishu"]
    }))
}

// 获取活跃会话
async fn api_active_sessions() -> Json<JsonValue> {
    Json(json!({
        "sessions": [],
        "count": 0
    }))
}

// 获取消息历史
async fn api_message_history(State(state): State<WebState>) -> Json<JsonValue> {
    let history = state.conversation_history.read().await;
    Json(json!({
        "messages": *history,
        "count": history.len()
    }))
}

// 获取用户统计
async fn api_user_stats() -> Json<JsonValue> {
    Json(json!({
        "total_users": 0,
        "active_users": 0,
        "top_users": []
    }))
}

// 审计管理页面
async fn audit_handler() -> Html<&'static str> {
    Html(HTML_AUDIT)
}

// ==================== 审计 API ====================

/// 审计日志查询参数
#[derive(Debug, Deserialize)]
pub struct AuditQueryParams {
    pub start_time: Option<u64>, // Unix timestamp
    pub end_time: Option<u64>,   // Unix timestamp
    pub level: Option<String>,
    pub channel: Option<String>,
    pub user_id: Option<String>,
    pub limit: Option<u32>,
}

/// 获取审计统计
async fn api_audit_stats(State(state): State<WebState>) -> Json<JsonValue> {
    let audit_guard = state.audit_service.read().await;
    if let Some(audit_service) = &*audit_guard {
        match audit_service.get_stats().await {
            Ok(stats) => Json(json!({
                "success": true,
                "stats": {
                    "total_events": stats.total_events,
                    "today_events": stats.today_events,
                    "warning_events": stats.warning_events,
                    "error_events": stats.error_events,
                    "by_level": stats.by_level,
                    "by_category": stats.by_category,
                    "by_channel": stats.by_channel,
                }
            })),
            Err(e) => Json(json!({
                "success": false,
                "error": e.to_string()
            })),
        }
    } else {
        Json(json!({
            "success": false,
            "error": "审计服务未启用"
        }))
    }
}

/// 查询审计日志
async fn api_audit_logs(
    State(state): State<WebState>,
    Query(params): Query<AuditQueryParams>,
) -> Json<JsonValue> {
    let audit_guard = state.audit_service.read().await;
    if let Some(audit_service) = &*audit_guard {
        let now = SystemTime::now();
        let start_time = params.start_time
            .map(|ts| SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(ts))
            .unwrap_or(now - std::time::Duration::from_secs(24 * 60 * 60)); // 默认 24 小时
        let end_time = params.end_time
            .map(|ts| SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(ts))
            .unwrap_or(now);

        let level = params.level.as_ref().and_then(|l| {
            match l.to_uppercase().as_str() {
                "DEBUG" => Some(super::security::AuditLevel::Debug),
                "INFO" => Some(super::security::AuditLevel::Info),
                "WARNING" => Some(super::security::AuditLevel::Warning),
                "ERROR" => Some(super::security::AuditLevel::Error),
                "CRITICAL" => Some(super::security::AuditLevel::Critical),
                _ => None,
            }
        });

        match audit_service.export_logs(start_time, end_time, level).await {
            Ok(json_data) => Json(json!({
                "success": true,
                "logs": serde_json::from_str::<Vec<JsonValue>>(&json_data).unwrap_or_default()
            })),
            Err(e) => Json(json!({
                "success": false,
                "error": e.to_string()
            })),
        }
    } else {
        Json(json!({
            "success": false,
            "error": "审计服务未启用"
        }))
    }
}

/// 导出审计日志
async fn api_audit_export(
    State(state): State<WebState>,
    Query(params): Query<AuditQueryParams>,
) -> axum::response::Response {
    let audit_guard = state.audit_service.read().await;
    if let Some(audit_service) = &*audit_guard {
        let now = SystemTime::now();
        let start_time = params.start_time
            .map(|ts| SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(ts))
            .unwrap_or(now - std::time::Duration::from_secs(30 * 24 * 60 * 60)); // 默认 30 天
        let end_time = params.end_time
            .map(|ts| SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(ts))
            .unwrap_or(now);

        match audit_service.export_logs(start_time, end_time, None).await {
            Ok(json_data) => {
                let response = axum::response::Json(json_data);
                let mut resp = response.into_response();
                resp.headers_mut().insert(
                    axum::http::header::CONTENT_DISPOSITION,
                    "attachment; filename=\"audit_logs.json\"".parse().unwrap()
                );
                resp
            }
            Err(e) => {
                axum::response::Json(json!({"error": e.to_string()})).into_response()
            }
        }
    } else {
        axum::response::Json(json!({"error": "审计服务未启用"})).into_response()
    }
}

/// 清理过期审计日志
async fn api_audit_cleanup(State(state): State<WebState>) -> Json<JsonValue> {
    let audit_guard = state.audit_service.write().await;
    if let Some(audit_service) = &*audit_guard {
        match audit_service.cleanup_expired_logs().await {
            Ok(count) => Json(json!({
                "success": true,
                "deleted_count": count
            })),
            Err(e) => Json(json!({
                "success": false,
                "error": e.to_string()
            })),
        }
    } else {
        Json(json!({
            "success": false,
            "error": "审计服务未启用"
        }))
    }
}

// ==================== Web 服务器 ====================

/// Web 服务器
#[derive(Clone)]
pub struct WebServer {
    /// 服务器端口
    port: u16,
    /// 服务器状态（使用 Arc 共享）
    state: Arc<WebState>,
}

impl WebServer {
    /// 创建新的 Web 服务器
    pub fn new(port: u16) -> Self {
        Self {
            port,
            state: Arc::new(WebState {
                id: format!("{:x}", std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis()),
                message_stats: Arc::new(RwLock::new(MessageStats::default())),
                conversation_history: Arc::new(RwLock::new(Vec::new())),
                config: Arc::new(tokio::sync::Mutex::new(None)),
                audit_service: Arc::new(RwLock::new(None)),
                service_status: Arc::new(RwLock::new(ServiceStatus::Initializing)),
                ai_engine_status: Arc::new(RwLock::new(AIEngineStatus::default())),
                channel_status: Arc::new(RwLock::new(HashMap::new())),
                auth_config: Arc::new(RwLock::new(AuthConfig::default())),
                ws_manager: Arc::new(WebSocketManager::new()),
            }),
        }
    }

    /// 配置认证
    pub fn configure_auth(&self, username: &str, password: &str) {
        let mut auth_config = futures::executor::block_on(self.state.auth_config.write());
        auth_config.enabled = true;
        auth_config.username = username.to_string();
        auth_config.password = password.to_string();
        tracing::info!(username = username, "认证已启用");
    }

    /// 禁用认证
    pub fn disable_auth(&self) {
        let mut auth_config = futures::executor::block_on(self.state.auth_config.write());
        auth_config.enabled = false;
        tracing::info!("认证已禁用");
    }

    /// 设置服务状态
    pub fn set_service_status(&self, status: ServiceStatus) {
        tracing::info!(status = ?status, "服务状态即将更新");
        let mut guard = futures::executor::block_on(self.state.service_status.write());
        *guard = status;
        // 使用 guard 的值来日志，避免移动后使用
        tracing::info!(status = ?*guard, "服务状态已更新");
    }

    /// 设置 AI 引擎状态
    pub fn set_ai_engine_status(&self, status: AIEngineStatus) {
        tracing::info!(is_running = status.is_running, "AI 引擎状态即将更新");
        let mut guard = futures::executor::block_on(self.state.ai_engine_status.write());
        *guard = status;
        // 使用 guard 的值来日志
        tracing::info!(is_running = guard.is_running, "AI 引擎状态已更新");
    }

    /// 更新渠道连接状态
    pub fn update_channel_status(&self, channel: &str, status: ChannelConnectionStatus) {
        let connected = status.connected;
        tracing::info!(channel = channel, connected = connected, "渠道状态已更新");
        let mut guard = futures::executor::block_on(self.state.channel_status.write());
        guard.insert(channel.to_string(), status);
    }

    /// 设置审计服务
    pub fn set_audit_service(&self, audit_service: super::security::AuditService) {
        let mut guard = futures::executor::block_on(self.state.audit_service.write());
        *guard = Some(audit_service);
    }

    /// 获取状态
    pub fn state(&self) -> Arc<WebState> {
        self.state.clone()
    }

    /// 获取内部状态引用（用于路由）
    fn state_ref(&self) -> &WebState {
        &*self.state
    }

    /// 创建 Axum 路由
    pub fn create_router(&self) -> Router {
        // 创建公共路由（不需要认证）
        let public_routes = Router::new()
            // 健康检查端点（无认证）
            .route("/health", get(health_handler))
            .route("/ready", get(ready_handler))
            .route("/metrics", get(metrics_handler))
            // WebSocket 端点（无认证，方便前端连接）
            .route("/ws", get(ws_handler))
            // WebSocket 状态
            .route("/api/ws/status", get(ws_status_handler))
            .with_state(self.state_ref().clone());

        // 创建受保护的路由（需要认证）
        // 创建认证中间件状态
        let auth_middleware_state = AuthMiddlewareState::new(self.state.auth_config.clone());

        let protected_routes = Router::new()
            // 静态页面
            .route("/", get(index_handler))
            .route("/debug", get(debug_handler))
            .route("/config", get(config_handler))
            .route("/operations", get(operations_handler))
            .route("/audit", get(audit_handler))
            // API - 调试监控
            .route("/api/debug/send", post(api_send_message))
            .route("/api/debug/ai", post(api_ai_test))
            .route("/api/debug/history", get(api_conversation_history))
            .route("/api/debug/stats", get(api_stats))
            .route("/api/debug/clear", post(api_clear_history))
            // API - 仪表盘
            .route("/api/dashboard/stats", get(api_dashboard_stats))
            // API - 服务状态
            .route("/api/status", get(api_status_handler))
            // API - 配置管理
            .route("/api/config", get(api_get_config))
            .route("/api/config", put(api_update_config))
            .route("/api/config/providers", get(api_get_providers))
            .route("/api/config/channels", get(api_get_channels))
            // API - 运营数据
            .route("/api/operations/sessions", get(api_active_sessions))
            .route("/api/operations/messages", get(api_message_history))
            .route("/api/operations/users", get(api_user_stats))
            // API - 审计管理
            .route("/api/audit/stats", get(api_audit_stats))
            .route("/api/audit/logs", get(api_audit_logs))
            .route("/api/audit/export", get(api_audit_export))
            .route("/api/audit/cleanup", post(api_audit_cleanup))
            // API - 认证管理
            .route("/api/auth/status", get(auth_status_handler))
            .route("/api/auth/logout", post(logout_handler))
            // 静态资源
            .route("/static/style.css", get(static_style_css))
            .route("/static/app.js", get(static_app_js))
            // 添加认证中间件（带状态）
            .layer(axum::middleware::from_fn_with_state(
                auth_middleware_state,
                auth_middleware,
            ))
            // 添加主状态
            .with_state(self.state_ref().clone());

        // 合并路由
        public_routes.merge(protected_routes)
    }

    /// 启动服务器
    pub async fn start(&self) -> Result<(), Box<dyn std::error::Error>> {
        let addr = std::net::SocketAddr::from(([0, 0, 0, 0], self.port));
        info!(port = self.port, "启动 Web 管理界面");

        let router = self.create_router();

        let listener = tokio::net::TcpListener::bind(addr).await?;
        axum::serve(listener, router)
            .await
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)
    }
}

// 静态文件处理器
async fn static_style_css() -> impl IntoResponse {
    ([("Content-Type", "text/css")], CSS_CONTENT)
}

async fn static_app_js() -> impl IntoResponse {
    ([("Content-Type", "application/javascript")], JS_CONTENT)
}

// ==================== 前端页面 ====================

// 首页 - 仪表盘风格
const HTML_INDEX: &str = r#"
<!DOCTYPE html>
<html lang="zh-CN">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Clawdbot 仪表盘</title>
    <link rel="stylesheet" href="/static/style.css">
    <style>
        /* 仪表盘特定样式 */
        .dashboard-header {
            display: flex;
            justify-content: space-between;
            align-items: center;
            margin-bottom: 20px;
        }
        .dashboard-header h1 {
            margin-bottom: 0;
        }
        .header-actions {
            display: flex;
            gap: 10px;
        }
        .refresh-btn {
            background: #28a745;
        }
        .refresh-btn:hover {
            background: #218838;
        }
        .stat-cards {
            display: grid;
            grid-template-columns: repeat(4, 1fr);
            gap: 20px;
            margin-bottom: 20px;
        }
        .stat-card {
            background: #fff;
            border-radius: 12px;
            padding: 20px;
            box-shadow: 0 2px 8px rgba(0,0,0,0.1);
            position: relative;
            overflow: hidden;
        }
        .stat-card::before {
            content: '';
            position: absolute;
            top: 0;
            left: 0;
            width: 4px;
            height: 100%;
            background: var(--card-accent, #007bff);
        }
        .stat-card.service { --card-accent: #28a745; }
        .stat-card.messages { --card-accent: #17a2b8; }
        .stat-card.sessions { --card-accent: #6f42c1; }
        .stat-card.ai { --card-accent: #fd7e14; }
        .stat-card .stat-icon {
            font-size: 2rem;
            margin-bottom: 10px;
        }
        .stat-card .stat-value {
            font-size: 2rem;
            font-weight: bold;
            color: #333;
            margin-bottom: 5px;
        }
        .stat-card .stat-label {
            color: #666;
            font-size: 0.9rem;
        }
        .stat-card .stat-status {
            position: absolute;
            top: 15px;
            right: 15px;
            width: 12px;
            height: 12px;
            border-radius: 50%;
            background: #dc3545;
        }
        .stat-card .stat-status.running {
            background: #28a745;
            box-shadow: 0 0 8px rgba(40,167,69,0.5);
        }
        .dashboard-grid {
            display: grid;
            grid-template-columns: 1fr 1fr;
            gap: 20px;
        }
        .dashboard-section {
            background: #fff;
            border-radius: 12px;
            padding: 20px;
            box-shadow: 0 2px 8px rgba(0,0,0,0.1);
        }
        .dashboard-section h2 {
            margin-bottom: 15px;
            padding-bottom: 10px;
            border-bottom: 1px solid #eee;
            display: flex;
            align-items: center;
            gap: 8px;
        }
        .channel-list {
            display: flex;
            flex-direction: column;
            gap: 10px;
        }
        .channel-item {
            display: flex;
            align-items: center;
            padding: 12px 15px;
            background: #f8f9fa;
            border-radius: 8px;
            transition: all 0.2s;
        }
        .channel-item:hover {
            background: #e9ecef;
        }
        .channel-icon {
            font-size: 1.5rem;
            margin-right: 15px;
        }
        .channel-info {
            flex: 1;
        }
        .channel-name {
            font-weight: 600;
            color: #333;
        }
        .channel-stats {
            font-size: 0.85rem;
            color: #666;
            margin-top: 3px;
        }
        .channel-status {
            padding: 5px 12px;
            border-radius: 20px;
            font-size: 0.8rem;
            font-weight: 500;
        }
        .channel-status.connected {
            background: #d4edda;
            color: #155724;
        }
        .channel-status.disconnected {
            background: #f8d7da;
            color: #721c24;
        }
        .message-list {
            display: flex;
            flex-direction: column;
            gap: 8px;
            max-height: 300px;
            overflow-y: auto;
        }
        .message-item {
            padding: 10px 12px;
            background: #f8f9fa;
            border-radius: 8px;
            display: flex;
            gap: 12px;
            align-items: flex-start;
        }
        .message-time {
            color: #999;
            font-size: 0.8rem;
            min-width: 45px;
        }
        .message-channel {
            font-size: 0.75rem;
            padding: 2px 8px;
            background: #e9ecef;
            border-radius: 4px;
            color: #666;
        }
        .message-user {
            font-weight: 600;
            color: #007bff;
            font-size: 0.9rem;
        }
        .message-content {
            color: #333;
            font-size: 0.9rem;
            margin-top: 3px;
            word-break: break-word;
        }
        .empty-message {
            text-align: center;
            padding: 30px;
            color: #999;
        }
        .quick-actions {
            display: flex;
            flex-wrap: wrap;
            gap: 10px;
        }
        .quick-action-btn {
            display: flex;
            align-items: center;
            gap: 8px;
            padding: 12px 20px;
            background: #f8f9fa;
            border: 1px solid #dee2e6;
            border-radius: 8px;
            color: #495057;
            text-decoration: none;
            transition: all 0.2s;
            cursor: pointer;
        }
        .quick-action-btn:hover {
            background: #007bff;
            color: #fff;
            border-color: #007bff;
        }
        .performance-metrics {
            display: grid;
            grid-template-columns: repeat(3, 1fr);
            gap: 15px;
        }
        .metric-item {
            text-align: center;
            padding: 15px;
            background: #f8f9fa;
            border-radius: 8px;
        }
        .metric-value {
            font-size: 1.5rem;
            font-weight: bold;
            color: #007bff;
        }
        .metric-label {
            font-size: 0.85rem;
            color: #666;
            margin-top: 5px;
        }
        @media (max-width: 1200px) {
            .stat-cards { grid-template-columns: repeat(2, 1fr); }
            .dashboard-grid { grid-template-columns: 1fr; }
        }
        @media (max-width: 768px) {
            .stat-cards { grid-template-columns: 1fr; }
        }
    </style>
</head>
<body>
    <nav class="sidebar">
        <h1>Clawdbot</h1>
        <ul>
            <li><a href="/" class="active">首页</a></li>
            <li><a href="/debug">调试监控</a></li>
            <li><a href="/config">配置管理</a></li>
            <li><a href="/operations">运营数据</a></li>
            <li><a href="/audit">安全审计</a></li>
        </ul>
        <div class="sidebar-footer">
            <button id="logoutBtn" class="logout-btn">退出登录</button>
        </div>
    </nav>
    <main>
        <div class="dashboard-header">
            <h1>Clawdbot 仪表盘</h1>
            <div class="header-actions">
                <button onclick="refreshDashboard()" class="refresh-btn">刷新数据</button>
            </div>
        </div>

        <!-- 统计卡片区 -->
        <div class="stat-cards">
            <div class="stat-card service">
                <div class="stat-icon">🟢</div>
                <div class="stat-status" id="serviceStatusIndicator"></div>
                <div class="stat-value" id="serviceStatusText">初始化中</div>
                <div class="stat-label">服务状态</div>
            </div>
            <div class="stat-card messages">
                <div class="stat-icon">💬</div>
                <div class="stat-value" id="todayMessages">0</div>
                <div class="stat-label">今日消息</div>
            </div>
            <div class="stat-card sessions">
                <div class="stat-icon">👥</div>
                <div class="stat-value" id="activeSessions">0</div>
                <div class="stat-label">活跃会话</div>
            </div>
            <div class="stat-card ai">
                <div class="stat-icon">🤖</div>
                <div class="stat-value" id="aiSuccessRate">--%</div>
                <div class="stat-label">AI 成功率</div>
            </div>
        </div>

        <!-- 主体网格 -->
        <div class="dashboard-grid">
            <!-- 渠道状态 -->
            <div class="dashboard-section">
                <h2>📡 渠道连接状态</h2>
                <div class="channel-list" id="channelList">
                    <div class="empty-message">加载中...</div>
                </div>
            </div>

            <!-- 最近消息 -->
            <div class="dashboard-section">
                <h2>💬 最近消息</h2>
                <div class="message-list" id="recentMessages">
                    <div class="empty-message">暂无消息</div>
                </div>
            </div>

            <!-- 性能指标 -->
            <div class="dashboard-section">
                <h2>⚡ 性能指标</h2>
                <div class="performance-metrics">
                    <div class="metric-item">
                        <div class="metric-value" id="avgResponseTime">--ms</div>
                        <div class="metric-label">平均响应时间</div>
                    </div>
                    <div class="metric-item">
                        <div class="metric-value" id="todayTokens">0</div>
                        <div class="metric-label">今日 Token</div>
                    </div>
                    <div class="metric-item">
                        <div class="metric-value" id="aiRequests">0</div>
                        <div class="metric-label">AI 请求数</div>
                    </div>
                </div>
            </div>

            <!-- 快捷操作 -->
            <div class="dashboard-section">
                <h2>🚀 快捷操作</h2>
                <div class="quick-actions">
                    <a href="/debug" class="quick-action-btn">📊 查看统计</a>
                    <a href="/config" class="quick-action-btn">⚙️ 配置管理</a>
                    <a href="/audit" class="quick-action-btn">📜 审计日志</a>
                    <a href="/operations" class="quick-action-btn">📈 运营数据</a>
                </div>
            </div>
        </div>
    </main>
    <script src="/static/app.js"></script>
    <script>
        // 加载仪表盘数据
        async function loadDashboard() {
            try {
                const response = await fetch('/api/dashboard/stats');
                const data = await response.json();

                // 更新服务状态
                const statusIndicator = document.getElementById('serviceStatusIndicator');
                const statusText = document.getElementById('serviceStatusText');
                if (data.service_status === 'running') {
                    statusIndicator.className = 'stat-status running';
                    statusText.textContent = '运行中';
                } else if (data.service_status === 'error') {
                    statusIndicator.className = 'stat-status';
                    statusText.textContent = '错误';
                } else {
                    statusIndicator.className = 'stat-status';
                    statusText.textContent = data.service_status;
                }

                // 更新统计卡片
                document.getElementById('todayMessages').textContent = data.today_messages || 0;
                document.getElementById('activeSessions').textContent = data.active_sessions || 0;
                document.getElementById('aiSuccessRate').textContent =
                    (data.ai_success_rate !== undefined ? data.ai_success_rate.toFixed(1) : '--') + '%';

                // 更新性能指标
                document.getElementById('avgResponseTime').textContent =
                    (data.avg_response_time_ms !== undefined ? Math.round(data.avg_response_time_ms) : '--') + 'ms';
                document.getElementById('todayTokens').textContent = formatNumber(data.today_tokens || 0);
                document.getElementById('aiRequests').textContent = formatNumber(
                    (data.ai_success_rate !== undefined && data.ai_success_rate > 0) ?
                    Math.round(data.today_messages / (data.ai_success_rate / 100)) : 0);

                // 更新渠道状态
                const channelList = document.getElementById('channelList');
                if (data.channels && data.channels.length > 0) {
                    channelList.innerHTML = data.channels.map(ch => `
                        <div class="channel-item">
                            <span class="channel-icon">${ch.icon}</span>
                            <div class="channel-info">
                                <div class="channel-name">${ch.display_name}</div>
                                <div class="channel-stats">${ch.message_count || 0} 条消息</div>
                            </div>
                            <span class="channel-status ${ch.connected ? 'connected' : 'disconnected'}">
                                ${ch.connected ? '已连接' : '未连接'}
                            </span>
                        </div>
                    `).join('');
                } else {
                    channelList.innerHTML = '<div class="empty-message">暂无渠道配置</div>';
                }

                // 更新最近消息
                const recentMessages = document.getElementById('recentMessages');
                if (data.recent_messages && data.recent_messages.length > 0) {
                    recentMessages.innerHTML = data.recent_messages.map(msg => `
                        <div class="message-item">
                            <span class="message-time">${msg.time}</span>
                            <div style="flex:1">
                                <div>
                                    <span class="message-channel">${msg.channel}</span>
                                    <span class="message-user">${escapeHtml(msg.user)}</span>
                                </div>
                                <div class="message-content">${escapeHtml(msg.content)}</div>
                            </div>
                        </div>
                    `).join('');
                } else {
                    recentMessages.innerHTML = '<div class="empty-message">暂无消息</div>';
                }
            } catch (e) {
                console.error('加载仪表盘数据失败:', e);
            }
        }

        // 刷新仪表盘
        function refreshDashboard() {
            loadDashboard();
        }

        // 格式化数字
        function formatNumber(num) {
            if (num >= 1000000) {
                return (num / 1000000).toFixed(1) + 'M';
            } else if (num >= 1000) {
                return (num / 1000).toFixed(1) + 'K';
            }
            return num.toString();
        }

        // 页面加载时初始化
        document.addEventListener('DOMContentLoaded', function() {
            loadDashboard();
            // 每30秒自动刷新
            setInterval(loadDashboard, 30000);
        });
    </script>
</body>
</html>
"#;

// 调试监控页面
const HTML_DEBUG: &str = r#"
<!DOCTYPE html>
<html lang="zh-CN">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>调试监控 - Clawdbot</title>
    <link rel="stylesheet" href="/static/style.css">
</head>
<body>
    <nav class="sidebar">
        <h1>Clawdbot</h1>
        <ul>
            <li><a href="/">首页</a></li>
            <li><a href="/debug" class="active">调试监控</a></li>
            <li><a href="/config">配置管理</a></li>
            <li><a href="/operations">运营数据</a></li>
            <li><a href="/audit">安全审计</a></li>
        </ul>
        <div class="sidebar-footer">
            <button id="logoutBtn" class="logout-btn">退出登录</button>
        </div>
    </nav>
    <main>
        <h1>调试监控</h1>

        <!-- 大模型调试 -->
        <section class="card">
            <h2>大模型调试</h2>
            <form id="aiTestForm">
                <div class="form-row">
                    <div class="form-group">
                        <label>模型</label>
                        <select id="aiModel" name="model">
                            <option value="minimax">MiniMax</option>
                            <option value="openai">OpenAI</option>
                            <option value="anthropic">Anthropic</option>
                        </select>
                    </div>
                    <div class="form-group">
                        <label>Temperature</label>
                        <input type="number" id="aiTemperature" name="temperature" value="0.7" step="0.1" min="0" max="2">
                    </div>
                    <div class="form-group">
                        <label>Max Tokens</label>
                        <input type="number" id="aiMaxTokens" name="maxTokens" value="4096" step="100">
                    </div>
                </div>
                <div class="form-group">
                    <label>系统提示词</label>
                    <textarea id="aiSystemPrompt" name="systemPrompt" rows="2" placeholder="可选的系统提示词"></textarea>
                </div>
                <div class="form-group">
                    <label>用户消息</label>
                    <textarea id="aiMessage" name="message" rows="3" placeholder="输入测试消息"></textarea>
                </div>
                <button type="submit">调用大模型</button>
            </form>
            <div id="aiResult" class="result-box"></div>
        </section>

        <!-- 渠道调试 -->
        <section class="card">
            <h2>渠道调试</h2>
            <form id="channelTestForm">
                <div class="form-row">
                    <div class="form-group">
                        <label>渠道</label>
                        <select id="channel" name="channel">
                            <option value="feishu">飞书</option>
                            <option value="telegram">Telegram</option>
                            <option value="discord">Discord</option>
                        </select>
                    </div>
                    <div class="form-group" style="flex:1">
                        <label>目标 ID</label>
                        <input type="text" id="targetId" name="targetId" placeholder="用户 ID 或群组 ID">
                    </div>
                </div>
                <div class="form-group">
                    <label>消息内容</label>
                    <textarea id="channelMessage" name="message" rows="3" placeholder="输入要发送的消息"></textarea>
                </div>
                <button type="submit">发送消息</button>
            </form>
            <div id="channelResult" class="result-box"></div>
        </section>

        <!-- 统计概览 -->
        <section class="card">
            <h2>统计概览</h2>
            <div class="stats-grid">
                <div class="stat-item">
                    <span class="stat-value" id="totalMessages">0</span>
                    <span class="stat-label">总消息数</span>
                </div>
                <div class="stat-item">
                    <span class="stat-value" id="todayMessages">0</span>
                    <span class="stat-label">今日消息</span>
                </div>
                <div class="stat-item">
                    <span class="stat-value" id="totalTokens">0</span>
                    <span class="stat-label">总 Token</span>
                </div>
                <div class="stat-item">
                    <span class="stat-value" id="todayTokens">0</span>
                    <span class="stat-label">今日 Token</span>
                </div>
            </div>
        </section>

        <!-- 对话历史 -->
        <section class="card">
            <h2>对话历史 <button onclick="clearHistory()" class="btn-small">清空</button></h2>
            <div id="historyList"></div>
        </section>
    </main>
    <script src="/static/app.js"></script>
</body>
</html>
"#;

// 配置管理页面
const HTML_CONFIG: &str = r#"
<!DOCTYPE html>
<html lang="zh-CN">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>配置管理 - Clawdbot</title>
    <link rel="stylesheet" href="/static/style.css">
</head>
<body>
    <nav class="sidebar">
        <h1>Clawdbot</h1>
        <ul>
            <li><a href="/">首页</a></li>
            <li><a href="/debug">调试监控</a></li>
            <li><a href="/config" class="active">配置管理</a></li>
            <li><a href="/operations">运营数据</a></li>
            <li><a href="/audit">安全审计</a></li>
        </ul>
        <div class="sidebar-footer">
            <button id="logoutBtn" class="logout-btn">退出登录</button>
        </div>
    </nav>
    <main>
        <h1>配置管理</h1>

        <section class="card">
            <h2>AI Provider 配置</h2>
            <div id="providersList"></div>
        </section>

        <section class="card">
            <h2>渠道配置</h2>
            <div id="channelsList"></div>
        </section>

        <section class="card">
            <h2>路由规则</h2>
            <div id="routingList">
                <p>暂无路由规则</p>
            </div>
        </section>
    </main>
    <script src="/static/app.js"></script>
</body>
</html>
"#;

// 运营数据页面
const HTML_OPERATIONS: &str = r#"
<!DOCTYPE html>
<html lang="zh-CN">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>运营数据 - Clawdbot</title>
    <link rel="stylesheet" href="/static/style.css">
</head>
<body>
    <nav class="sidebar">
        <h1>Clawdbot</h1>
        <ul>
            <li><a href="/">首页</a></li>
            <li><a href="/debug">调试监控</a></li>
            <li><a href="/config">配置管理</a></li>
            <li><a href="/operations" class="active">运营数据</a></li>
            <li><a href="/audit">安全审计</a></li>
        </ul>
        <div class="sidebar-footer">
            <button id="logoutBtn" class="logout-btn">退出登录</button>
        </div>
    </nav>
    <main>
        <h1>运营数据</h1>

        <section class="card">
            <h2>活跃会话</h2>
            <div id="sessionsList">
                <p>暂无活跃会话</p>
            </div>
        </section>

        <section class="card">
            <h2>消息统计</h2>
            <div id="messageStats"></div>
        </section>

        <section class="card">
            <h2>用户统计</h2>
            <div id="userStats"></div>
        </section>
    </main>
    <script src="/static/app.js"></script>
</body>
</html>
"#;

// 安全审计页面
const HTML_AUDIT: &str = r#"
<!DOCTYPE html>
<html lang="zh-CN">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>安全审计 - Clawdbot</title>
    <link rel="stylesheet" href="/static/style.css">
    <style>
        .audit-level { padding: 3px 8px; border-radius: 3px; font-size: 12px; }
        .level-debug { background: #e9ecef; color: #495057; }
        .level-info { background: #cce5ff; color: #004085; }
        .level-warning { background: #fff3cd; color: #856404; }
        .level-error { background: #f8d7da; color: #721c24; }
        .level-critical { background: #dc3545; color: #fff; }
        .log-table { width: 100%; border-collapse: collapse; }
        .log-table th, .log-table td { padding: 10px; text-align: left; border-bottom: 1px solid #eee; }
        .log-table th { background: #f8f9fa; font-weight: 600; }
        .log-table tr:hover { background: #f8f9fa; }
        .filter-bar { display: flex; gap: 10px; margin-bottom: 20px; flex-wrap: wrap; }
        .filter-bar select, .filter-bar input { padding: 8px; border: 1px solid #ddd; border-radius: 4px; }
        .export-btn { background: #28a745; }
        .cleanup-btn { background: #dc3545; }
    </style>
</head>
<body>
    <nav class="sidebar">
        <h1>Clawdbot</h1>
        <ul>
            <li><a href="/">首页</a></li>
            <li><a href="/debug">调试监控</a></li>
            <li><a href="/config">配置管理</a></li>
            <li><a href="/operations">运营数据</a></li>
            <li><a href="/audit" class="active">安全审计</a></li>
        </ul>
        <div class="sidebar-footer">
            <button id="logoutBtn" class="logout-btn">退出登录</button>
        </div>
    </nav>
    <main>
        <h1>安全审计</h1>

        <section class="card">
            <h2>审计统计</h2>
            <div class="stats-grid">
                <div class="stat-item">
                    <span class="stat-value" id="totalEvents">0</span>
                    <span class="stat-label">总事件数</span>
                </div>
                <div class="stat-item">
                    <span class="stat-value" id="todayEvents">0</span>
                    <span class="stat-label">今日事件</span>
                </div>
                <div class="stat-item">
                    <span class="stat-value" id="warningEvents">0</span>
                    <span class="stat-label">警告事件</span>
                </div>
                <div class="stat-item">
                    <span class="stat-value" id="errorEvents">0</span>
                    <span class="stat-label">错误事件</span>
                </div>
            </div>
        </section>

        <section class="card">
            <h2>
                审计日志
                <button onclick="exportLogs()" class="btn-small export-btn">导出 JSON</button>
                <button onclick="cleanupLogs()" class="btn-small cleanup-btn">清理过期</button>
            </h2>
            <div class="filter-bar">
                <select id="filterLevel">
                    <option value="">所有级别</option>
                    <option value="DEBUG">调试</option>
                    <option value="INFO">信息</option>
                    <option value="WARNING">警告</option>
                    <option value="ERROR">错误</option>
                    <option value="CRITICAL">严重</option>
                </select>
                <input type="text" id="filterChannel" placeholder="渠道筛选">
                <input type="text" id="filterUser" placeholder="用户筛选">
                <button onclick="loadLogs()">查询</button>
            </div>
            <div id="logsTable"></div>
        </section>
    </main>
    <script src="/static/app.js"></script>
    <script>
// 加载审计统计
async function loadAuditStats() {
    const result = await api('/api/audit/stats');
    if (result.success) {
        document.getElementById('totalEvents').textContent = result.stats.total_events || 0;
        document.getElementById('todayEvents').textContent = result.stats.today_events || 0;
        document.getElementById('warningEvents').textContent = result.stats.warning_events || 0;
        document.getElementById('errorEvents').textContent = result.stats.error_events || 0;
    }
}

// 加载审计日志
async function loadLogs() {
    const level = document.getElementById('filterLevel').value;
    const channel = document.getElementById('filterChannel').value;
    const user = document.getElementById('filterUser').value;

    let url = '/api/audit/logs?';
    if (level) url += 'level=' + level + '&';
    if (channel) url += 'channel=' + encodeURIComponent(channel) + '&';
    if (user) url += 'user_id=' + encodeURIComponent(user) + '&';

    const result = await api(url);
    const container = document.getElementById('logsTable');

    if (result.success && result.logs.length > 0) {
        let html = '<table class="log-table"><thead><tr><th>时间</th><th>级别</th><th>渠道</th><th>用户</th><th>内容</th></tr></thead><tbody>';
        result.logs.forEach(log => {
            const levelClass = 'level-' + (log.level || 'info').toLowerCase();
            html += '<tr>';
            html += '<td>' + (log.timestamp || '') + '</td>';
            html += '<td><span class="audit-level ' + levelClass + '">' + (log.level || 'INFO') + '</span></td>';
            html += '<td>' + (log.channel || '') + '</td>';
            html += '<td>' + (log.user_id || log.channel || '') + '</td>';
            html += '<td>' + (log.content || '').substring(0, 100) + '</td>';
            html += '</tr>';
        });
        html += '</tbody></table>';
        container.innerHTML = html;
    } else {
        container.innerHTML = '<p>暂无审计日志</p>';
    }
}

// 导出审计日志
async function exportLogs() {
    window.location.href = '/api/audit/export';
}

// 清理过期日志
async function cleanupLogs() {
    if (confirm('确定要清理 30 天前的审计日志吗？')) {
        const result = await api('/api/audit/cleanup', {method: 'POST'});
        if (result.success) {
            alert('已清理 ' + result.deleted_count + ' 条日志');
            loadAuditStats();
            loadLogs();
        } else {
            alert('清理失败: ' + result.error);
        }
    }
}

// 页面加载时初始化
if (document.getElementById('logsTable')) {
    loadAuditStats();
    loadLogs();
}
    </script>
</body>
</html>
"#;

// CSS 样式
const CSS_CONTENT: &str = r#"
* { box-sizing: border-box; margin: 0; padding: 0; }
body { font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif; display: flex; min-height: 100vh; background: #f5f5f5; }
.sidebar { width: 200px; background: #1a1a2e; color: #fff; padding: 20px; }
.sidebar h1 { font-size: 1.5rem; margin-bottom: 20px; }
.sidebar ul { list-style: none; }
.sidebar li { margin: 10px 0; }
.sidebar a { color: #ccc; text-decoration: none; display: block; padding: 10px; border-radius: 5px; }
.sidebar a:hover, .sidebar a.active { background: #16213e; color: #fff; }
main { flex: 1; padding: 20px; overflow-y: auto; }
h1 { margin-bottom: 20px; color: #333; }
h2 { margin-bottom: 15px; color: #555; font-size: 1.1rem; }
.card { background: #fff; border-radius: 8px; padding: 20px; margin-bottom: 20px; box-shadow: 0 2px 4px rgba(0,0,0,0.1); }
.form-group { margin-bottom: 15px; }
.form-group label { display: block; margin-bottom: 5px; font-weight: 500; }
.form-group input, .form-group select, .form-group textarea { width: 100%; padding: 10px; border: 1px solid #ddd; border-radius: 4px; font-size: 14px; }
button { background: #007bff; color: #fff; border: none; padding: 10px 20px; border-radius: 4px; cursor: pointer; }
button:hover { background: #0056b3; }
.btn-small { padding: 5px 10px; font-size: 12px; background: #6c757d; }
.stats-grid { display: grid; grid-template-columns: repeat(4, 1fr); gap: 15px; }
.stat-item { text-align: center; padding: 15px; background: #f8f9fa; border-radius: 8px; }
.stat-value { display: block; font-size: 2rem; font-weight: bold; color: #007bff; }
.stat-label { color: #666; font-size: 0.9rem; }
#historyList, #providersList, #channelsList, #routingList { max-height: 400px; overflow-y: auto; }
.history-item, .provider-item, .channel-item, .routing-item { padding: 10px; border-bottom: 1px solid #eee; }
.history-item:last-child { border-bottom: none; }
.msg-user { color: #007bff; font-weight: 500; }
.msg-bot { color: #28a745; }
.msg-content { margin: 5px 0; }
.timestamp { color: #999; font-size: 0.8rem; }
#sendResult { margin-top: 15px; padding: 10px; border-radius: 4px; }
#sendResult.success { background: #d4edda; color: #155724; }
#sendResult.error { background: #f8d7da; color: #721c24; }

/* 侧边栏底部 */
.sidebar-footer { margin-top: auto; padding-top: 20px; border-top: 1px solid rgba(255,255,255,0.1); }

/* 退出登录按钮 */
.logout-btn { width: 100%; background: #dc3545; color: #fff; border: none; padding: 10px 20px; border-radius: 4px; cursor: pointer; font-size: 14px; }
.logout-btn:hover { background: #c82333; }

/* 表单行布局 */
.form-row { display: flex; gap: 15px; }
.form-row .form-group { flex: 1; margin-bottom: 15px; }

/* 结果展示框 */
.result-box { margin-top: 15px; padding: 15px; border-radius: 4px; background: #f8f9fa; border: 1px solid #ddd; }
.result-box.success { background: #d4edda; color: #155724; border-color: #c3e6cb; }
.result-box.error { background: #f8d7da; color: #721c24; border-color: #f5c6cb; }
.result-box.loading { background: #fff3cd; color: #856404; border-color: #ffeeba; }
.result-box pre { white-space: pre-wrap; word-wrap: break-word; margin: 10px 0 0 0; font-family: monospace; font-size: 13px; }
.result-meta { font-size: 12px; color: #666; margin-top: 10px; padding-top: 10px; border-top: 1px solid #ddd; }
"#;

// JavaScript
const JS_CONTENT: &str = r#"
async function api(url, options = {}) {
    const response = await fetch(url, {
        headers: {'Content-Type': 'application/json'},
        ...options
    });
    return response.json();
}

// 大模型调试 - 调用大模型
document.getElementById('aiTestForm')?.addEventListener('submit', async (e) => {
    e.preventDefault();
    const model = document.getElementById('aiModel').value;
    const temperature = parseFloat(document.getElementById('aiTemperature').value) || 0.7;
    const maxTokens = parseInt(document.getElementById('aiMaxTokens').value) || 4096;
    const systemPrompt = document.getElementById('aiSystemPrompt').value;
    const message = document.getElementById('aiMessage').value;

    const resultDiv = document.getElementById('aiResult');
    resultDiv.className = 'result-box loading';
    resultDiv.innerHTML = '正在调用大模型...';

    try {
        const result = await api('/api/debug/ai', {
            method: 'POST',
            body: JSON.stringify({model, temperature, max_tokens: maxTokens, system_prompt: systemPrompt, message})
        });

        if (result.success) {
            resultDiv.className = 'result-box success';
            let html = '<strong>响应：</strong>';
            html += '<pre>' + escapeHtml(result.response) + '</pre>';
            if (result.usage) {
                html += '<div class="result-meta">';
                html += 'Token: ' + result.usage.total_tokens + ' (输入: ' + result.usage.prompt_tokens + ', 输出: ' + result.usage.completion_tokens + ')';
                html += ' | 耗时: ' + (result.response_time_ms || 0) + 'ms';
                html += '</div>';
            }
            resultDiv.innerHTML = html;
        } else {
            resultDiv.className = 'result-box error';
            resultDiv.innerHTML = '<strong>错误：</strong><pre>' + escapeHtml(result.error) + '</pre>';
        }
    } catch (e) {
        resultDiv.className = 'result-box error';
        resultDiv.innerHTML = '<strong>异常：</strong><pre>' + escapeHtml(e.message) + '</pre>';
    }
});

// 渠道调试 - 发送消息
document.getElementById('channelTestForm')?.addEventListener('submit', async (e) => {
    e.preventDefault();
    const channel = document.getElementById('channel').value;
    const targetId = document.getElementById('targetId').value;
    const message = document.getElementById('channelMessage').value;

    const resultDiv = document.getElementById('channelResult');
    resultDiv.className = 'result-box loading';
    resultDiv.innerHTML = '正在发送消息...';

    try {
        const result = await api('/api/debug/send', {
            method: 'POST',
            body: JSON.stringify({channel, target_id: targetId, message})
        });

        if (result.success) {
            resultDiv.className = 'result-box success';
            let html = '<strong>发送成功！</strong>';
            if (result.message_id) {
                html += '<div class="result-meta">消息ID: ' + result.message_id + '</div>';
            }
            resultDiv.innerHTML = html;
            loadHistory();
            loadStats();
        } else {
            resultDiv.className = 'result-box error';
            resultDiv.innerHTML = '<strong>发送失败：</strong><pre>' + escapeHtml(result.error) + '</pre>';
        }
    } catch (e) {
        resultDiv.className = 'result-box error';
        resultDiv.innerHTML = '<strong>异常：</strong><pre>' + escapeHtml(e.message) + '</pre>';
    }
});

// HTML 转义
function escapeHtml(text) {
    if (!text) return '';
    const div = document.createElement('div');
    div.textContent = text;
    return div.innerHTML;
}

// 加载统计
async function loadStats() {
    const stats = await api('/api/debug/stats');
    document.getElementById('totalMessages').textContent = stats.total_messages || 0;
    document.getElementById('todayMessages').textContent = stats.today_messages || 0;
    document.getElementById('totalTokens').textContent = stats.total_tokens || 0;
    document.getElementById('todayTokens').textContent = stats.today_tokens || 0;
}

// 加载历史
async function loadHistory() {
    const history = await api('/api/debug/history');
    const container = document.getElementById('historyList');
    if (history.length === 0) {
        container.innerHTML = '<p>暂无对话记录</p>';
        return;
    }
    container.innerHTML = history.map(h => '<div class="history-item"><div class="timestamp">' + new Date(h.timestamp).toLocaleString() + '</div><div><span class="msg-user">[' + h.channel + ']</span>: ' + escapeHtml(h.message) + '</div><div class="msg-bot">→ ' + escapeHtml(h.response) + '</div></div>').join('');
}

// 清除历史
async function clearHistory() {
    await api('/api/debug/clear', {method: 'POST'});
    loadHistory();
    loadStats();
}

// 页面加载时初始化
if (document.getElementById('historyList')) loadHistory();
if (document.getElementById('totalMessages')) loadStats();
if (document.getElementById('providersList')) loadProviders();
if (document.getElementById('channelsList')) loadChannels();

async function loadProviders() {
    const data = await api('/api/config/providers');
    document.getElementById('providersList').innerHTML = data.providers.map(p => '<div class="provider-item"><strong>' + p + '</strong>' + (p === data.default ? ' (默认)' : '') + '</div>').join('');
}

async function loadChannels() {
    const data = await api('/api/config/channels');
    document.getElementById('channelsList').innerHTML = data.channels.map(c => '<div class="channel-item"><strong>' + c + '</strong> ' + (data.enabled.includes(c) ? '✓ 已启用' : '✗ 已禁用') + '</div>').join('');
}

// 退出登录按钮点击事件
document.getElementById('logoutBtn')?.addEventListener('click', async () => {
    if (confirm('确定要退出登录吗？')) {
        try {
            await fetch('/api/auth/logout', { method: 'POST' });
            // 清除认证信息并刷新页面或跳转到首页
            sessionStorage.removeItem('auth');
            window.location.reload();
        } catch (e) {
            console.error('退出失败:', e);
            window.location.reload();
        }
    }
});
"#;
