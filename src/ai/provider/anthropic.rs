//! Anthropic AI Provider 实现
//!
//! 本模块实现了 Anthropic 的 Claude 模型接口。
//!
//! # 功能
//! - 聊天完成
//! - 流式响应
//! - 嵌入向量
//!
//! # 配置文件示例
//! ```toml
//! [ai.providers.anthropic]
//! api_key = "${ANTHROPIC_API_KEY}"
//! model = "claude-3-5-sonnet-20241022"
//! temperature = 0.7
//! max_tokens = 4096
//! ```

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{debug, error, info, warn};

use super::{AiProvider, ChatMessage, ChatRequest, ChatResponse, ModelConfig, ProviderRegistry, TokenUsage};
use crate::infra::error::Result;
use crate::ai::constants::{
    ANTHROPIC_BASE_URL, ANTHROPIC_DEFAULT_MODEL,
    DEFAULT_TEMPERATURE, DEFAULT_MAX_TOKENS, DEFAULT_TIMEOUT,
    ANTHROPIC_API_VERSION, PROVIDER_ANTHROPIC,
};

/// Anthropic Provider 配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicConfig {
    /// API Key
    pub api_key: String,
    /// API Base URL
    pub base_url: Option<String>,
    /// 模型名称
    pub model: Option<String>,
    /// 温度参数
    pub temperature: Option<f32>,
    /// 最大 Token 数
    pub max_tokens: Option<u32>,
    /// API 版本
    pub api_version: Option<String>,
}

impl Default for AnthropicConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            base_url: None,
            model: Some(ANTHROPIC_DEFAULT_MODEL.to_string()),
            temperature: Some(DEFAULT_TEMPERATURE),
            max_tokens: Some(DEFAULT_MAX_TOKENS),
            api_version: Some(ANTHROPIC_API_VERSION.to_string()),
        }
    }
}

/// Anthropic 聊天请求
#[derive(Debug, Serialize)]
struct AnthropicChatRequest {
    /// 模型名称
    model: String,
    /// 消息列表
    messages: Vec<AnthropicMessage>,
    /// 最大 Token 数
    max_tokens: u32,
    /// 温度参数
    temperature: Option<f32>,
    /// 系统提示词
    system: Option<String>,
}

/// Anthropic 消息
#[derive(Debug, Serialize)]
struct AnthropicMessage {
    /// 角色
    role: String,
    /// 内容
    content: String,
}

/// Anthropic 聊天响应
#[derive(Debug, Deserialize)]
struct AnthropicChatResponse {
    /// 响应 ID
    id: String,
    /// 类型
    #[serde(rename = "type")]
    response_type: String,
    /// 角色
    role: String,
    /// 内容块
    content: Vec<AnthropicContentBlock>,
    /// 停止原因
    stop_reason: Option<String>,
    /// 使用统计
    usage: AnthropicUsage,
}

/// Anthropic 内容块
#[derive(Debug, Deserialize)]
struct AnthropicContentBlock {
    /// 类型
    #[serde(rename = "type")]
    block_type: String,
    /// 文本内容
    text: Option<String>,
}

/// Anthropic 使用统计
#[derive(Debug, Deserialize)]
struct AnthropicUsage {
    /// 输入 Token 数
    #[serde(rename = "input_tokens")]
    input_tokens: u32,
    /// 输出 Token 数
    #[serde(rename = "output_tokens")]
    output_tokens: u32,
}

/// Anthropic Provider
///
/// 实现 Anthropic Claude 模型的 AI Provider
#[derive(Debug, Clone)]
pub struct AnthropicProvider {
    /// 配置
    config: AnthropicConfig,
    /// HTTP 客户端
    http_client: reqwest::Client,
}

impl AnthropicProvider {
    /// 创建新的 Anthropic Provider
    ///
    /// # 参数说明
    /// * `config` - Provider 配置
    ///
    /// # 返回值
    /// 创建的 Provider
    pub fn new(config: AnthropicConfig) -> Self {
        let http_client = reqwest::Client::builder()
            .timeout(DEFAULT_TIMEOUT)
            .build()
            .expect("创建 HTTP 客户端失败");

        Self {
            config,
            http_client,
        }
    }

    /// 获取 API Base URL
    fn get_base_url(&self) -> String {
        self.config.base_url.clone()
            .unwrap_or_else(|| ANTHROPIC_BASE_URL.to_string())
    }

    /// 获取模型名称
    fn get_model(&self) -> String {
        self.config.model.clone()
            .unwrap_or_else(|| ANTHROPIC_DEFAULT_MODEL.to_string())
    }
}

#[async_trait::async_trait]
impl AiProvider for AnthropicProvider {
    /// 获取 Provider 名称
    fn name(&self) -> &str {
        PROVIDER_ANTHROPIC
    }

    /// 发送聊天请求
    async fn chat(&self, request: &ChatRequest) -> Result<ChatResponse> {
        let model = self.get_model();
        let base_url = self.get_base_url();

        debug!(model = %model, "发送 Anthropic 聊天请求");

        // 构建 Anthropic 格式的消息
        let mut messages: Vec<AnthropicMessage> = Vec::new();

        // 添加系统消息（如果有）
        let system_prompt = request.model.system_prompt.clone()
            .or(request.messages.iter()
                .find(|m| m.role == super::MessageRole::System)
                .map(|m| m.content.clone()));

        // 转换消息格式
        for msg in &request.messages {
            if msg.role == super::MessageRole::System {
                continue; // 系统消息单独处理
            }

            let role = match msg.role {
                super::MessageRole::User => "user",
                super::MessageRole::Assistant => "assistant",
                super::MessageRole::Tool => "user", // Tool 消息转为 user
                _ => "user",
            };

            messages.push(AnthropicMessage {
                role: role.to_string(),
                content: msg.content.clone(),
            });
        }

        // 构建请求
        let chat_request = AnthropicChatRequest {
            model: model.clone(),
            messages,
            max_tokens: request.model.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS),
            temperature: request.model.temperature.or(self.config.temperature),
            system: system_prompt,
        };

        // 发送请求
        let response = self.http_client
            .post(format!("{}/v1/messages", base_url))
            .header("x-api-key", &self.config.api_key)
            .header("anthropic-version", self.config.api_version.clone()
                .unwrap_or_else(|| ANTHROPIC_API_VERSION.to_string()))
            .header("content-type", "application/json")
            .json(&chat_request)
            .send()
            .await
            .map_err(|e| crate::infra::error::Error::Ai(format!("Anthropic API 请求失败: {}", e)))?;

        // 检查响应状态
        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            error!(status = ?status, error = %error_text, "Anthropic API 错误");
            return Err(crate::infra::error::Error::Ai(format!("Anthropic API 错误: {}", error_text)));
        }

        // 解析响应
        let response_body: AnthropicChatResponse = response
            .json()
            .await
            .map_err(|e| crate::infra::error::Error::Ai(format!("解析 Anthropic 响应失败: {}", e)))?;

        // 提取文本内容
        let content = response_body.content
            .iter()
            .filter_map(|block| block.text.clone())
            .collect::<Vec<_>>()
            .join("\n");

        let usage = TokenUsage {
            prompt_tokens: response_body.usage.input_tokens,
            completion_tokens: response_body.usage.output_tokens,
            total_tokens: response_body.usage.input_tokens + response_body.usage.output_tokens,
        };

        info!(
            model = %model,
            prompt_tokens = usage.prompt_tokens,
            completion_tokens = usage.completion_tokens,
            "Anthropic 聊天响应"
        );

        Ok(ChatResponse {
            id: response_body.id,
            content,
            usage,
            done: response_body.stop_reason.is_some(),
        })
    }

    /// 发送流式聊天请求
    async fn chat_stream(
        &self,
        _request: &ChatRequest,
    ) -> Result<Box<dyn tokio::io::AsyncRead + Send + Unpin>> {
        // TODO: 实现流式响应
        // 需要使用 SSE (Server-Sent Events) 解析
        Err(crate::infra::error::Error::Ai("流式响应尚未实现".to_string()))
    }

    /// 创建嵌入向量
    async fn embeddings(&self, texts: &[String], _model: &str) -> Result<Vec<Vec<f32>>> {
        // Anthropic 目前不支持嵌入向量
        // 可以考虑使用 OpenAI 的嵌入 API
        warn!("Anthropic 不支持嵌入向量");
        Err(crate::infra::error::Error::Ai("Anthropic 不支持嵌入向量".to_string()))
    }
}

/// 将 ChatRequest 转换为 Anthropic 格式
fn convert_message(msg: &ChatMessage) -> AnthropicMessage {
    let role = match msg.role {
        super::MessageRole::User => "user",
        super::MessageRole::Assistant => "assistant",
        super::MessageRole::System => "user", // 系统消息需要特殊处理
        super::MessageRole::Tool => "user",
    };

    AnthropicMessage {
        role: role.to_string(),
        content: msg.content.clone(),
    }
}

/// 注册 Anthropic Provider 到注册表
pub fn register_provider(registry: &ProviderRegistry, config: AnthropicConfig) {
    let provider = AnthropicProvider::new(config);
    registry.register(provider);
}
