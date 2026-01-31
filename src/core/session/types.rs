//! 会话类型定义
//!
//! 定义会话相关的核心数据结构。

use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime};
use std::collections::HashMap;
use uuid::Uuid;
use crate::core::message::types::MessageContent;

/// 会话状态
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionStatus {
    /// 活跃会话 - 正在对话中
    Active,
    /// 暂停会话 - 暂时无活动
    Paused,
    /// 已结束会话
    Ended,
    /// 已过期会话
    Expired,
}

/// 会话配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionConfig {
    /// 会话过期时间（秒）
    pub expire_seconds: u64,
    /// 最大历史消息数
    pub max_history: usize,
    /// 是否持久化到数据库
    pub persist_enabled: bool,
    /// 数据库路径
    pub db_path: String,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            expire_seconds: 3600,      // 1小时过期
            max_history: 50,           // 最多50条历史消息
            persist_enabled: true,     // 默认启用持久化（使用 SQLite）
            db_path: "data/clawdbot.db".to_string(), // 默认数据库路径
        }
    }
}

/// 会话信息
///
/// 代表一个完整的对话会话，包含元数据和消息历史
///
/// # 字段说明
/// * `id` - 唯一会话ID
/// * `channel` - 渠道类型（如 "feishu"）
/// * `target_id` - 目标ID（用户ID或群组ID）
/// * `agent_id` - 使用的Agent ID
/// * `status` - 会话状态
/// * `created_at` - 创建时间
/// * `updated_at` - 最后活动时间
/// * `metadata` - 附加元数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    /// 唯一会话ID
    pub id: String,
    /// 渠道类型
    pub channel: String,
    /// 目标ID（用户或群组）
    pub target_id: String,
    /// 发送者ID
    pub sender_id: Option<String>,
    /// 使用的Agent ID
    pub agent_id: String,
    /// 会话状态
    pub status: SessionStatus,
    /// 创建时间戳
    pub created_at: i64,
    /// 最后更新时间戳
    pub updated_at: i64,
    /// 消息历史
    pub messages: Vec<SessionMessage>,
    /// 附加元数据
    pub metadata: HashMap<String, String>,
}

impl Session {
    /// 创建新会话
    ///
    /// # 参数说明
    /// * `channel` - 渠道类型
    /// * `target_id` - 目标ID
    /// * `sender_id` - 发送者ID
    /// * `agent_id` - Agent ID
    pub fn new(channel: &str, target_id: &str, sender_id: Option<&str>, agent_id: &str) -> Self {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or(Duration::from_secs(0))
            .as_millis() as i64;

        Self {
            id: format!("sess_{}", Uuid::new_v4().to_string().replace('-', "")),
            channel: channel.to_string(),
            target_id: target_id.to_string(),
            sender_id: sender_id.map(|s| s.to_string()),
            agent_id: agent_id.to_string(),
            status: SessionStatus::Active,
            created_at: now,
            updated_at: now,
            messages: Vec::new(),
            metadata: HashMap::new(),
        }
    }

    /// 从唯一键创建会话（用于查找）
    pub fn from_key(channel: &str, target_id: &str, sender_id: Option<&str>) -> String {
        match sender_id {
            Some(sender) => format!("{}:{}:{}", channel, target_id, sender),
            None => format!("{}:{}", channel, target_id),
        }
    }

    /// 生成会话唯一键
    pub fn generate_session_key(&self) -> String {
        Self::from_key(&self.channel, &self.target_id, self.sender_id.as_deref())
    }

    /// 添加消息到历史
    ///
    /// # 参数说明
    /// * `role` - 消息角色
    /// * `content` - 消息内容
    pub fn add_message(&mut self, role: &str, content: &str) {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or(Duration::from_secs(0))
            .as_millis() as i64;

        self.messages.push(SessionMessage {
            id: format!("msg_{}", Uuid::new_v4().to_string().replace('-', "")),
            role: role.to_string(),
            content: content.to_string(),
            created_at: now,
        });

        // 更新最后活动时间
        self.updated_at = now;
    }

    /// 检查会话是否过期
    pub fn is_expired(&self, expire_seconds: u64) -> bool {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or(Duration::from_secs(0))
            .as_millis() as i64;

        let elapsed = (now - self.updated_at) / 1000; // 转换为秒
        elapsed > expire_seconds as i64
    }

    /// 截断历史消息（保留最新的）
    pub fn truncate_history(&mut self, max_count: usize) {
        if self.messages.len() > max_count {
            let keep_count = self.messages.len().saturating_sub(max_count);
            self.messages = self.messages[keep_count..].to_vec();
        }
    }
}

/// 会话中的单条消息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMessage {
    /// 消息ID
    pub id: String,
    /// 角色（user/assistant/system）
    pub role: String,
    /// 内容
    pub content: String,
    /// 创建时间戳
    pub created_at: i64,
}

/// 会话查询条件
#[derive(Debug, Clone, Default)]
pub struct SessionQuery {
    /// 渠道类型
    pub channel: Option<String>,
    /// 目标ID
    pub target_id: Option<String>,
    /// 发送者ID
    pub sender_id: Option<String>,
    /// Agent ID
    pub agent_id: Option<String>,
    /// 状态
    pub status: Option<SessionStatus>,
    /// 包含过期会话
    pub include_expired: bool,
    /// 最大结果数
    pub limit: usize,
}

impl SessionQuery {
    /// 创建新查询
    pub fn new() -> Self {
        Self::default()
    }

    /// 设置渠道
    pub fn channel(mut self, channel: &str) -> Self {
        self.channel = Some(channel.to_string());
        self
    }

    /// 设置目标ID
    pub fn target_id(mut self, target_id: &str) -> Self {
        self.target_id = Some(target_id.to_string());
        self
    }

    /// 设置发送者ID
    pub fn sender_id(mut self, sender_id: &str) -> Self {
        self.sender_id = Some(sender_id.to_string());
        self
    }

    /// 设置Agent ID
    pub fn agent_id(mut self, agent_id: &str) -> Self {
        self.agent_id = Some(agent_id.to_string());
        self
    }

    /// 设置状态
    pub fn status(mut self, status: SessionStatus) -> Self {
        self.status = Some(status);
        self
    }

    /// 设置限制
    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }
}

/// 会话统计信息
#[derive(Debug, Clone, Default)]
pub struct SessionStats {
    /// 总会话数
    pub total_sessions: u64,
    /// 活跃会话数
    pub active_sessions: u64,
    /// 今日新增会话数
    pub today_new_sessions: u64,
    /// 平均会话消息数
    pub avg_messages_per_session: f64,
}
