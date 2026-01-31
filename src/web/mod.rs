//! Web 管理界面模块
//!
//! 提供 Web 管理界面和 API 接口。

use axum::{
    routing::{get, post, put},
    Router,
    extract::{State, Query},
    response::{Html, IntoResponse},
    Json,
};
use std::sync::Arc;
use std::time::SystemTime;
use tokio::sync::RwLock;
use tracing::{info, error};
use serde::{Serialize, Deserialize};
use serde_json::{Value as JsonValue, json};
use std::collections::HashMap;

// ==================== 类型定义 ====================

/// Web 服务器状态
#[derive(Clone)]
pub struct WebState {
    /// 消息统计
    pub message_stats: Arc<RwLock<MessageStats>>,
    /// 对话历史（内存中）
    pub conversation_history: Arc<RwLock<Vec<Conversation>>>,
    /// 配置
    #[allow(dead_code)]
    pub config: Arc<tokio::sync::Mutex<Option<super::infra::config::Config>>>,
    /// 审计服务（可选）
    pub audit_service: Arc<RwLock<Option<super::security::AuditService>>>,
}

/// 消息统计
#[derive(Debug, Default, Clone, Serialize)]
pub struct MessageStats {
    pub total_messages: u64,
    pub today_messages: u64,
    pub total_tokens: u64,
    pub today_tokens: u64,
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

// 发送测试消息 - 调用真实 MiniMax AI
async fn api_send_message(
    State(state): State<WebState>,
    Json(req): Json<SendMessageRequest>,
) -> Json<SendMessageResponse> {
    info!(channel = %req.channel, target = %req.target_id, message = %req.message, "收到测试消息请求");

    // 调用 MiniMax AI
    let ai_response = call_minimax_ai(&req.message).await;

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

            Json(SendMessageResponse {
                success: true,
                message_id: Some(uuid::Uuid::new_v4().to_string()),
                response: Some(response.content),
                error: None,
            })
        }
        Err(e) => {
            error!(error = %e, "AI 调用失败");
            Json(SendMessageResponse {
                success: false,
                message_id: None,
                response: None,
                error: Some(e.to_string()),
            })
        }
    }
}

/// 调用 MiniMax AI
async fn call_minimax_ai(message: &str) -> Result<super::ai::provider::ChatResponse, String> {
    use super::ai::provider::AiProvider;

    // 从环境变量加载配置
    let api_key = std::env::var("MINIMAX_API_KEY")
        .map_err(|_| "MINIMAX_API_KEY 未设置")?;
    let group_id = std::env::var("MINIMAX_GROUP_ID")
        .unwrap_or_else(|_| "default".to_string());
    let model = std::env::var("MINIMAX_MODEL")
        .unwrap_or_else(|_| "MiniMax-M2.1".to_string());
    let temperature = std::env::var("MINIMAX_TEMPERATURE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.7);
    let max_tokens = std::env::var("MINIMAX_MAX_TOKENS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(4096);

    info!(model = %model, "使用 MiniMax 模型");

    // 创建 MiniMax 配置
    let config = super::ai::provider::minimax::MiniMaxConfig {
        api_key,
        group_id,
        model: Some(model.clone()),
        temperature: Some(temperature),
        max_tokens: Some(max_tokens),
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
            temperature: Some(temperature),
            max_tokens: Some(max_tokens),
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
    /// 服务器状态
    state: WebState,
}

impl WebServer {
    /// 创建新的 Web 服务器
    pub fn new(port: u16) -> Self {
        Self {
            port,
            state: WebState {
                message_stats: Arc::new(RwLock::new(MessageStats::default())),
                conversation_history: Arc::new(RwLock::new(Vec::new())),
                config: Arc::new(tokio::sync::Mutex::new(None)),
                audit_service: Arc::new(RwLock::new(None)),
            },
        }
    }

    /// 设置审计服务
    pub fn set_audit_service(&self, audit_service: super::security::AuditService) {
        let mut guard = futures::executor::block_on(self.state.audit_service.write());
        *guard = Some(audit_service);
    }

    /// 获取状态
    pub fn state(&self) -> &WebState {
        &self.state
    }

    /// 创建 Axum 路由
    pub fn create_router(&self) -> Router {
        Router::new()
            // 静态页面
            .route("/", get(index_handler))
            .route("/debug", get(debug_handler))
            .route("/config", get(config_handler))
            .route("/operations", get(operations_handler))
            .route("/audit", get(audit_handler))
            // API - 调试监控
            .route("/api/debug/send", post(api_send_message))
            .route("/api/debug/history", get(api_conversation_history))
            .route("/api/debug/stats", get(api_stats))
            .route("/api/debug/clear", post(api_clear_history))
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
            // 静态资源
            .route("/static/style.css", get(static_style_css))
            .route("/static/app.js", get(static_app_js))
            .with_state(self.state.clone())
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

// 首页
const HTML_INDEX: &str = r#"
<!DOCTYPE html>
<html lang="zh-CN">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Clawdbot 管理界面</title>
    <link rel="stylesheet" href="/static/style.css">
</head>
<body>
    <nav class="sidebar">
        <h1>Clawdbot</h1>
        <ul>
            <li><a href="/">首页</a></li>
            <li><a href="/debug">调试监控</a></li>
            <li><a href="/config">配置管理</a></li>
            <li><a href="/operations">运营数据</a></li>
        </ul>
    </nav>
    <main>
        <h1>欢迎使用 Clawdbot</h1>
        <p>选择左侧菜单开始使用</p>
    </main>
    <script src="/static/app.js"></script>
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
        </ul>
    </nav>
    <main>
        <h1>调试监控</h1>

        <section class="card">
            <h2>发送测试消息</h2>
            <form id="sendForm">
                <div class="form-group">
                    <label>渠道</label>
                    <select id="channel" name="channel">
                        <option value="feishu">飞书</option>
                    </select>
                </div>
                <div class="form-group">
                    <label>目标 ID</label>
                    <input type="text" id="targetId" name="targetId" placeholder="用户或群组 ID">
                </div>
                <div class="form-group">
                    <label>消息内容</label>
                    <textarea id="message" name="message" rows="3" placeholder="输入测试消息"></textarea>
                </div>
                <button type="submit">发送</button>
            </form>
            <div id="sendResult"></div>
        </section>

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
        </ul>
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
        </ul>
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

// 发送消息
document.getElementById('sendForm')?.addEventListener('submit', async (e) => {
    e.preventDefault();
    const channel = document.getElementById('channel').value;
    const targetId = document.getElementById('targetId').value;
    const message = document.getElementById('message').value;

    const result = await api('/api/debug/send', {
        method: 'POST',
        body: JSON.stringify({channel, target_id: targetId, message})
    });

    const resultDiv = document.getElementById('sendResult');
    if (result.success) {
        resultDiv.className = 'success';
        resultDiv.innerHTML = '发送成功！响应：' + result.response;
        loadHistory();
        loadStats();
    } else {
        resultDiv.className = 'error';
        resultDiv.innerHTML = '失败：' + result.error;
    }
});

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
    container.innerHTML = history.map(h => '<div class="history-item"><div class="timestamp">' + new Date(h.timestamp).toLocaleString() + '</div><div><span class="msg-user">[' + h.channel + ']</span>: ' + h.message + '</div><div class="msg-bot">→ ' + h.response + '</div></div>').join('');
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
"#;
