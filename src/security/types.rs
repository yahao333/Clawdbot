//! 安全审计类型定义
//!
//! 定义审计事件、级别、分类等核心类型

use serde::{Deserialize, Serialize};
use std::time::SystemTime;

/// 审计级别
///
/// 表示审计事件的重要程度（从低到高排序）
#[derive(Debug, Clone, PartialEq, PartialOrd, Serialize, Deserialize)]
pub enum AuditLevel {
    /// 调试信息
    Debug,
    /// 普通信息
    Info,
    /// 警告信息
    Warning,
    /// 错误信息
    Error,
    /// 严重告警
    Critical,
}

impl std::fmt::Display for AuditLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuditLevel::Debug => write!(f, "DEBUG"),
            AuditLevel::Info => write!(f, "INFO"),
            AuditLevel::Warning => write!(f, "WARNING"),
            AuditLevel::Error => write!(f, "ERROR"),
            AuditLevel::Critical => write!(f, "CRITICAL"),
        }
    }
}

/// 审计类别
///
/// 表示审计事件的来源分类
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AuditCategory {
    /// 入站消息
    InboundMessage,
    /// 出站消息
    OutboundMessage,
    /// API 调用
    ApiCall,
    /// 用户操作
    UserAction,
    /// 认证事件
    Authentication,
    /// 系统事件
    System,
    /// 安全事件
    Security,
    /// 飞书渠道
    ChannelFeishu,
    /// 其他渠道
    ChannelOther,
}

impl std::fmt::Display for AuditCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuditCategory::InboundMessage => write!(f, "INBOUND_MESSAGE"),
            AuditCategory::OutboundMessage => write!(f, "OUTBOUND_MESSAGE"),
            AuditCategory::ApiCall => write!(f, "API_CALL"),
            AuditCategory::UserAction => write!(f, "USER_ACTION"),
            AuditCategory::Authentication => write!(f, "AUTHENTICATION"),
            AuditCategory::System => write!(f, "SYSTEM"),
            AuditCategory::Security => write!(f, "SECURITY"),
            AuditCategory::ChannelFeishu => write!(f, "CHANNEL_FEISHU"),
            AuditCategory::ChannelOther => write!(f, "CHANNEL_OTHER"),
        }
    }
}

/// 审计上下文
///
/// 包含审计事件发生时的上下文信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditContext {
    /// 消息 ID
    pub message_id: String,
    /// 渠道类型（如：feishu, telegram）
    pub channel: String,
    /// 用户/发送者 ID
    pub user_id: String,
    /// 目标 ID（如：群聊 ID）
    pub target_id: String,
    /// 消息类型
    pub message_type: String,
    /// 客户端 IP（用于网络请求审计）
    pub client_ip: Option<String>,
    /// 会话 ID
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Agent ID（用于标识处理此消息的 AI Agent）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// AI 提供商（如：deepseek, openai, minimax）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ai_provider: Option<String>,
    /// 使用的 AI 模型
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ai_model: Option<String>,
    /// 输入 Token 数量
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_tokens: Option<u32>,
    /// 输出 Token 数量
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completion_tokens: Option<u32>,
    /// 处理耗时（毫秒）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// 路由置信度
    #[serde(skip_serializing_if = "Option::is_none")]
    pub routing_confidence: Option<f64>,
    /// 消息是否安全（通过敏感词检测）
    pub is_safe: bool,
    /// 检测到的敏感词列表
    pub detected_words: Vec<String>,
}

impl AuditContext {
    /// 创建审计上下文
    ///
    /// # 参数说明
    /// * `channel` - 渠道类型
    /// * `user_id` - 用户 ID
    /// * `target_id` - 目标 ID
    /// * `message_type` - 消息类型
    pub fn new(channel: &str, user_id: &str, target_id: &str, message_type: &str) -> Self {
        Self {
            message_id: uuid::Uuid::new_v4().to_string(),
            channel: channel.to_string(),
            user_id: user_id.to_string(),
            target_id: target_id.to_string(),
            message_type: message_type.to_string(),
            client_ip: None,
            session_id: None,
            agent_id: None,
            ai_provider: None,
            ai_model: None,
            prompt_tokens: None,
            completion_tokens: None,
            duration_ms: None,
            routing_confidence: None,
            is_safe: true,
            detected_words: vec![],
        }
    }

    /// 从消息创建审计上下文
    pub fn from_message(message: &crate::core::message::types::InboundMessage) -> Self {
        // 根据消息内容推断消息类型
        let message_type = if message.content.text.is_some() {
            "text".to_string()
        } else if !message.content.attachments.is_empty() {
            "attachment".to_string()
        } else if message.content.rich_text.is_some() {
            "rich_text".to_string()
        } else {
            "unknown".to_string()
        };

        Self {
            message_id: message.id.clone(),
            channel: message.channel.clone(),
            user_id: message.sender.id.clone(),
            target_id: message.target.id.clone(),
            message_type,
            client_ip: None,
            session_id: None,
            agent_id: None,
            ai_provider: None,
            ai_model: None,
            prompt_tokens: None,
            completion_tokens: None,
            duration_ms: None,
            routing_confidence: None,
            is_safe: true,
            detected_words: vec![],
        }
    }
}

/// 审计事件
///
/// 记录一次审计事件的所有信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    /// 事件唯一 ID
    pub id: String,
    /// 事件发生时间
    pub timestamp: SystemTime,
    /// 审计级别
    pub level: AuditLevel,
    /// 审计类别
    pub category: AuditCategory,
    /// 事件类型（具体操作）
    pub event_type: String,
    /// 审计上下文
    pub context: AuditContext,
    /// 事件内容（已脱敏）
    pub content: String,
    /// 原始内容（可选，仅在配置允许时存储）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_content: Option<String>,
    /// 额外元数据
    pub metadata: serde_json::Value,
}

/// 审计查询条件
///
/// 用于查询审计日志
#[derive(Debug, Clone, Default)]
pub struct AuditQuery {
    /// 开始时间
    pub start_time: Option<SystemTime>,
    /// 结束时间
    pub end_time: Option<SystemTime>,
    /// 审计级别
    pub level: Option<AuditLevel>,
    /// 审计类别
    pub category: Option<AuditCategory>,
    /// 用户 ID
    pub user_id: Option<String>,
    /// 渠道
    pub channel: Option<String>,
    /// 事件类型
    pub event_type: Option<String>,
    /// 关键字搜索
    pub keyword: Option<String>,
    /// 分页大小
    pub limit: u32,
    /// 分页偏移
    pub offset: u32,
}

impl AuditQuery {
    /// 创建新查询
    pub fn new() -> Self {
        Self::default()
    }

    /// 设置时间范围
    pub fn time_range(mut self, start: SystemTime, end: SystemTime) -> Self {
        self.start_time = Some(start);
        self.end_time = Some(end);
        self
    }

    /// 设置级别过滤
    pub fn with_level(mut self, level: AuditLevel) -> Self {
        self.level = Some(level);
        self
    }

    /// 设置类别过滤
    pub fn with_category(mut self, category: AuditCategory) -> Self {
        self.category = Some(category);
        self
    }

    /// 设置用户过滤
    pub fn with_user(mut self, user_id: &str) -> Self {
        self.user_id = Some(user_id.to_string());
        self
    }

    /// 设置关键字搜索
    pub fn with_keyword(mut self, keyword: &str) -> Self {
        self.keyword = Some(keyword.to_string());
        self
    }

    /// 设置分页
    pub fn paginate(mut self, offset: u32, limit: u32) -> Self {
        self.offset = offset;
        self.limit = limit;
        self
    }
}

/// 审计统计信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditStats {
    /// 总事件数
    pub total_events: u64,
    /// 按级别统计
    pub by_level: serde_json::Value,
    /// 按类别统计
    pub by_category: serde_json::Value,
    /// 按渠道统计
    pub by_channel: serde_json::Value,
    /// 今日事件数
    pub today_events: u64,
    /// 警告事件数
    pub warning_events: u64,
    /// 错误事件数
    pub error_events: u64,
}

/// 告警配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertConfig {
    /// 是否启用告警
    pub enabled: bool,
    /// 告警级别阈值（只有此级别及以上才发送告警）
    pub level_threshold: AuditLevel,
    /// 邮件服务器地址
    pub smtp_host: String,
    /// 邮件服务器端口
    pub smtp_port: u16,
    /// 发件人邮箱
    pub smtp_from: String,
    /// 发件人密码
    pub smtp_password: String,
    /// 收件人列表
    pub recipients: Vec<String>,
    /// 告警冷却时间（秒）
    pub cooldown_seconds: u64,
}

impl Default for AlertConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            level_threshold: AuditLevel::Warning,
            smtp_host: "smtp.example.com".to_string(),
            smtp_port: 587,
            smtp_from: "noreply@example.com".to_string(),
            smtp_password: "".to_string(),
            recipients: vec![],
            cooldown_seconds: 300,
        }
    }
}

/// 审计配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditConfig {
    /// 是否启用审计
    pub enabled: bool,
    /// 是否存储原始内容
    pub audit_store_original: bool,
    /// 审计日志保留天数
    pub audit_retention_days: u32,
    /// 告警配置
    pub alert: AlertConfig,
}

impl Default for AuditConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            audit_store_original: false,
            audit_retention_days: 30,
            alert: AlertConfig::default(),
        }
    }
}
