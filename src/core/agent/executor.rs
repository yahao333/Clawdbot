//! Agent 执行器模块
//!
//! 负责调用 AI 模型生成响应。

use tracing::{debug, info, instrument};

use crate::core::message::types::{InboundMessage, MessageContent};
use crate::infra::error::Result;

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

/// 默认 AI 引擎实现
///
/// 标准 AI 引擎实现
#[derive(Clone, Debug)]
pub struct DefaultAiEngine {
    // TODO: 实现 AI 引擎
}

impl DefaultAiEngine {
    /// 创建新的 AI 引擎
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait::async_trait]
impl AiEngine for DefaultAiEngine {
    #[instrument(skip(self, message), fields(agent_id))]
    async fn execute(
        &self,
        agent_id: &str,
        message: &InboundMessage,
    ) -> Result<MessageContent> {
        debug!(agent_id = agent_id, "开始执行 Agent");

        // TODO: 实现实际的 AI 调用
        // 这里需要：
        // 1. 获取 Agent 配置
        // 2. 构建消息历史
        // 3. 调用 AI Provider
        // 4. 返回响应

        // 模拟响应（实际实现时移除）
        let text = message.content.text.clone()
            .or(message.content.rich_text.as_ref().map(|rt| rt.content.clone()))
            .unwrap_or_else(String::new);
        let response_text = format!("收到消息: {}", text);

        info!(agent_id = agent_id, "Agent 执行完成");

        Ok(MessageContent::text(&response_text))
    }

    async fn execute_stream(
        &self,
        agent_id: &str,
        message: &InboundMessage,
    ) -> Result<Box<dyn tokio::io::AsyncRead + Send + Unpin>> {
        // TODO: 实现流式响应
        Err(AiEngineError::AgentNotFound {
            agent_id: agent_id.to_string(),
        }.into())
    }
}
