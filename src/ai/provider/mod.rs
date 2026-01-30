//! AI Provider 接口模块
//!
//! 定义 AI Provider 的统一接口。

// 子模块
pub mod anthropic;
pub mod openai;
pub mod minimax;

use std::sync::Arc;
use serde::{Deserialize, Serialize};

use crate::infra::error::Result;
use crate::core::message::types::MessageContent;

/// 消息角色
///
/// 定义消息在对话中的角色
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageRole {
    /// 系统消息
    System,
    /// 用户消息
    User,
    /// 助手消息
    Assistant,
    /// 工具调用
    Tool,
}

/// 聊天消息
///
/// 单条对话消息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    /// 消息角色
    pub role: MessageRole,
    /// 消息内容
    pub content: String,
    /// 消息名称（可选）
    pub name: Option<String>,
}

/// 聊天请求
///
/// 发送给 AI 的请求
#[derive(Debug, Clone)]
pub struct ChatRequest {
    /// 模型配置
    pub model: ModelConfig,
    /// 消息历史
    pub messages: Vec<ChatMessage>,
    /// 工具定义（可选）
    pub tools: Vec<ToolDefinition>,
    /// 是否流式输出
    pub stream: bool,
}

/// 模型配置
///
/// AI 模型的配置参数
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    /// Provider 名称
    pub provider: String,
    /// 模型名称
    pub model: String,
    /// API Key（敏感）
    pub api_key: Option<String>,
    /// Base URL（可选）
    pub base_url: Option<String>,
    /// 温度参数（0.0 - 2.0）
    pub temperature: Option<f32>,
    /// 最大 Token 数
    pub max_tokens: Option<u32>,
    /// 系统提示词
    pub system_prompt: Option<String>,
}

/// 工具定义
///
/// 定义可用的工具（用于 Function Calling）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    /// 工具名称
    pub name: String,
    /// 工具描述
    pub description: String,
    /// 参数模式（JSON Schema）
    pub parameters: serde_json::Value,
}

/// 聊天响应
///
/// AI 的响应
#[derive(Debug, Clone)]
pub struct ChatResponse {
    /// 响应 ID
    pub id: String,
    /// 响应内容
    pub content: String,
    /// 使用的 Token 数
    pub usage: TokenUsage,
    /// 是否完成
    pub done: bool,
}

/// Token 使用统计
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    /// 提示词 Token 数
    pub prompt_tokens: u32,
    /// 完成 Token 数
    pub completion_tokens: u32,
    /// 总 Token 数
    pub total_tokens: u32,
}

/// AI Provider Trait
///
/// 定义 AI Provider 的统一接口
///
/// # 实现要求
/// - 必须实现 `Send + Sync`
/// - 所有方法必须是异步的
#[async_trait::async_trait]
pub trait AiProvider: Send + Sync {
    /// 获取 Provider 名称
    fn name(&self) -> &str;

    /// 发送聊天请求
    async fn chat(&self, request: &ChatRequest) -> Result<ChatResponse>;

    /// 发送流式聊天请求
    async fn chat_stream(
        &self,
        request: &ChatRequest,
    ) -> Result<Box<dyn tokio::io::AsyncRead + Send + Unpin>>;

    /// 创建嵌入向量
    async fn embeddings(&self, texts: &[String], model: &str) -> Result<Vec<Vec<f32>>>;
}

/// AI Provider 注册表
///
/// 管理所有注册的 Provider
#[derive(Clone)]
pub struct ProviderRegistry {
    /// Provider 映射
    providers: Arc<dashmap::DashMap<String, Arc<dyn AiProvider>>>,
}

impl std::fmt::Debug for ProviderRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderRegistry")
            .field("providers_count", &self.providers.len())
            .finish()
    }
}

impl ProviderRegistry {
    /// 创建新的注册表
    pub fn new() -> Self {
        Self {
            providers: Arc::new(dashmap::DashMap::new()),
        }
    }

    /// 注册 Provider
    pub fn register<P: AiProvider + 'static>(&self, provider: P) {
        let name = provider.name().to_string();
        self.providers.insert(name.clone(), Arc::new(provider));
        tracing::info!(provider = name, "Provider 注册成功");
    }

    /// 获取 Provider
    pub fn get(&self, name: &str) -> Option<Arc<dyn AiProvider>> {
        self.providers.get(name).map(|p| p.clone())
    }

    /// 检查 Provider 是否存在
    pub fn contains(&self, name: &str) -> bool {
        self.providers.contains_key(name)
    }

    /// 列出所有 Provider
    pub fn list(&self) -> Vec<String> {
        self.providers.iter().map(|e| e.key().clone()).collect()
    }
}

impl Default for ProviderRegistry {
    fn default() -> Self {
        Self::new()
    }
}
