//! Agent 上下文模块
//!
//! 管理 Agent 执行时的上下文信息，包括：
//! 1. 消息历史
//! 2. 系统提示词
//! 3. 运行时状态
//!
//! # 使用示例
//! ```rust
//! let context = AgentContext::new("agent_001");
//! context.add_message(MessageRole::User, "你好");
//! let history = context.get_history();
//! ```

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::Arc;

/// 消息角色
///
/// 定义消息在对话中的角色
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageRole {
    /// 系统消息（定义 Agent 行为）
    System,
    /// 用户消息
    User,
    /// AI 助手消息
    Assistant,
    /// 工具调用消息
    Tool,
}

/// 对话消息
///
/// 记录单条对话消息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    /// 消息角色
    pub role: MessageRole,
    /// 消息内容
    pub content: String,
    /// 消息时间戳
    pub timestamp: DateTime<Utc>,
}

/// Agent 配置
///
/// 定义 Agent 的行为参数
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    /// Agent 唯一 ID
    pub id: String,
    /// Agent 名称
    pub name: String,
    /// 系统提示词
    pub system_prompt: String,
    /// AI 模型名称
    pub model: String,
    /// 温度参数（0.0 - 2.0）
    pub temperature: Option<f32>,
    /// 最大 Token 数量
    pub max_tokens: Option<u32>,
    /// 上下文历史长度
    pub history_limit: Option<u32>,
}

/// Agent 上下文
///
/// 管理 Agent 执行时的状态和信息
///
/// # 字段说明
/// * `config` - Agent 配置
/// * `message_history` - 消息历史
/// * `created_at` - 创建时间
#[derive(Clone, Debug)]
pub struct AgentContext {
    /// Agent 配置
    config: Arc<AgentConfig>,
    /// 消息历史（使用双端队列，支持高效追加和移除）
    message_history: Arc<parking_lot::Mutex<VecDeque<ChatMessage>>>,
    /// 创建时间
    created_at: DateTime<Utc>,
    /// 最后活跃时间
    last_active: Arc<parking_lot::Mutex<DateTime<Utc>>>,
}

impl AgentContext {
    /// 创建新的 Agent 上下文
    ///
    /// # 参数说明
    /// * `config` - Agent 配置
    ///
    /// # 返回值
    /// 创建的上下文
    pub fn new(config: AgentConfig) -> Self {
        let now = Utc::now();
        Self {
            config: Arc::new(config),
            message_history: Arc::new(parking_lot::Mutex::new(VecDeque::new())),
            created_at: now,
            last_active: Arc::new(parking_lot::Mutex::new(now)),
        }
    }

    /// 添加消息到历史
    ///
    /// # 参数说明
    /// * `role` - 消息角色
    /// * `content` - 消息内容
    pub fn add_message(&self, role: MessageRole, content: &str) {
        let message = ChatMessage {
            role,
            content: content.to_string(),
            timestamp: Utc::now(),
        };

        let mut history = self.message_history.lock();
        history.push_back(message);

        // 限制历史长度
        let limit = self.config.history_limit.unwrap_or(20) as usize;
        while history.len() > limit {
            history.pop_front();
        }

        // 更新最后活跃时间
        *self.last_active.lock() = Utc::now();
    }

    /// 获取消息历史
    ///
    /// # 返回值
    /// 消息历史的只读副本
    pub fn get_history(&self) -> Vec<ChatMessage> {
        self.message_history.lock().clone().into_iter().collect()
    }

    /// 获取消息历史长度
    ///
    /// # 返回值
    /// 历史消息数量
    pub fn history_len(&self) -> usize {
        self.message_history.lock().len()
    }

    /// 清空消息历史
    pub fn clear_history(&self) {
        self.message_history.lock().clear();
    }

    /// 获取 Agent 配置
    ///
    /// # 返回值
    /// Agent 配置的只读引用
    pub fn config(&self) -> &AgentConfig {
        &self.config
    }

    /// 获取创建时间
    ///
    /// # 返回值
    /// 创建时间
    pub fn created_at(&self) -> DateTime<Utc> {
        self.created_at
    }

    /// 获取最后活跃时间
    ///
    /// # 返回值
    /// 最后活跃时间
    pub fn last_active(&self) -> DateTime<Utc> {
        *self.last_active.lock()
    }
}

/// Agent 存储
///
/// 管理和缓存 Agent 上下文
#[derive(Clone, Debug)]
pub struct AgentStore {
    /// Agent 上下文存储
    agents: Arc<dashmap::DashMap<String, AgentContext>>,
}

impl AgentStore {
    /// 创建新的 Agent 存储
    pub fn new() -> Self {
        Self {
            agents: Arc::new(dashmap::DashMap::new()),
        }
    }

    /// 注册 Agent
    ///
    /// # 参数说明
    /// * `config` - Agent 配置
    ///
    /// # 返回值
    /// 创建的 Agent 上下文
    pub fn register(&self, config: AgentConfig) -> AgentContext {
        let context = AgentContext::new(config);
        self.agents.insert(context.config.id.clone(), context.clone());
        context
    }

    /// 获取 Agent 上下文
    ///
    /// # 参数说明
    /// * `agent_id` - Agent ID
    ///
    /// # 返回值
    /// Agent 上下文（如果存在）
    pub fn get(&self, agent_id: &str) -> Option<AgentContext> {
        self.agents.get(agent_id).map(|ctx| ctx.clone())
    }

    /// 检查 Agent 是否存在
    ///
    /// # 参数说明
    /// * `agent_id` - Agent ID
    ///
    /// # 返回值
    /// 是否存在
    pub fn contains(&self, agent_id: &str) -> bool {
        self.agents.contains_key(agent_id)
    }

    /// 列出所有 Agent
    ///
    /// # 返回值
    /// Agent ID 列表
    pub fn list(&self) -> Vec<String> {
        self.agents.iter().map(|e| e.key().clone()).collect()
    }

    /// 移除 Agent
    ///
    /// # 参数说明
    /// * `agent_id` - Agent ID
    pub fn remove(&self, agent_id: &str) {
        self.agents.remove(agent_id);
    }
}

impl Default for AgentStore {
    fn default() -> Self {
        Self::new()
    }
}
