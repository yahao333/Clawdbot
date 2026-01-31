//! Agent 执行器模块
//!
//! 负责调用 AI 模型生成响应。

use tracing::{debug, info, instrument};

use crate::core::message::types::{InboundMessage, MessageContent};
use crate::infra::error::Result;
use crate::ai::engine::AiEngine as CoreAiEngine;

/// AI 引擎 Trait
///
/// 定义 AI 引擎的接口
///
/// # 实现要求
/// - 必须实现 `Send + Sync` 以支持多线程
/// - 所有方法必须是异步的
#[async_trait::async_trait]
pub trait AiEngine: Send + Sync {
    /// 执行 Agent 生成响应
    ///
    /// # 参数说明
    /// * `agent_id` - Agent ID
    /// * `message` - 输入消息
    ///
    /// # 返回值
    /// 生成的响应内容
    async fn execute(
        &self,
        agent_id: &str,
        message: &InboundMessage,
    ) -> Result<MessageContent>;

    /// 执行流式响应
    ///
    /// # 参数说明
    /// * `agent_id` - Agent ID
    /// * `message` - 输入消息
    ///
    /// # 返回值
    /// 异步迭代器，逐块返回响应
    async fn execute_stream(
        &self,
        agent_id: &str,
        message: &InboundMessage,
    ) -> Result<Box<dyn tokio::io::AsyncRead + Send + Unpin>>;
}

/// AI 引擎错误
#[derive(Debug, thiserror::Error)]
pub enum AiEngineError {
    /// Agent 不存在
    #[error("Agent 不存在: {agent_id}")]
    AgentNotFound { agent_id: String },

    /// AI API 调用失败
    #[error("AI API 调用失败: {msg}")]
    ApiCallFailed { msg: String },

    /// 流式响应错误
    #[error("流式响应错误: {msg}")]
    StreamError { msg: String },
}

impl From<super::super::super::infra::error::Error> for AiEngineError {
    fn from(e: super::super::super::infra::error::Error) -> Self {
        Self::ApiCallFailed {
            msg: e.to_string(),
        }
    }
}

impl From<AiEngineError> for super::super::super::infra::error::Error {
    fn from(e: AiEngineError) -> Self {
        match e {
            AiEngineError::AgentNotFound { agent_id } => {
                super::super::super::infra::error::Error::Ai(format!("Agent 不存在: {}", agent_id))
            }
            AiEngineError::ApiCallFailed { msg } => {
                super::super::super::infra::error::Error::Ai(format!("AI API 调用失败: {}", msg))
            }
            AiEngineError::StreamError { msg } => {
                super::super::super::infra::error::Error::Ai(format!("流式响应错误: {}", msg))
            }
        }
    }
}

/// AI 响应
///
/// AI 模型生成的响应
#[derive(Debug, Clone)]
pub struct AiResponse {
    /// 响应文本内容
    pub content: String,
    /// 使用的 Token 数量
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
    /// 响应是否完成
    pub done: bool,
}

#[async_trait::async_trait]
impl AiEngine for CoreAiEngine {
    #[instrument(skip(self, message), fields(agent_id))]
    async fn execute(
        &self,
        agent_id: &str,
        message: &InboundMessage,
    ) -> Result<MessageContent> {
        self.execute(agent_id, message).await
    }

    async fn execute_stream(
        &self,
        agent_id: &str,
        message: &InboundMessage,
    ) -> Result<Box<dyn tokio::io::AsyncRead + Send + Unpin>> {
        self.execute_stream(agent_id, message).await
    }
}

/// 默认 AI 引擎实现（已弃用，请使用 CoreAiEngine）
#[derive(Clone, Debug)]
pub struct DefaultAiEngine {
    inner: CoreAiEngine,
}

impl DefaultAiEngine {
    /// 创建新的 AI 引擎
    pub fn new(config: &crate::infra::config::Config) -> Self {
        Self {
            inner: CoreAiEngine::new(config),
        }
    }
}

#[async_trait::async_trait]
impl AiEngine for DefaultAiEngine {
    async fn execute(
        &self,
        agent_id: &str,
        message: &InboundMessage,
    ) -> Result<MessageContent> {
        self.inner.execute(agent_id, message).await
    }

    async fn execute_stream(
        &self,
        agent_id: &str,
        message: &InboundMessage,
    ) -> Result<Box<dyn tokio::io::AsyncRead + Send + Unpin>> {
        self.inner.execute_stream(agent_id, message).await
    }
}
