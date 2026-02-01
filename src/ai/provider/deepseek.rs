//! DeepSeek AI Provider 实现
//!
//! 本模块实现了 DeepSeek 的 API 接口。
//! DeepSeek API 兼容 OpenAI 格式。
//!
//! # 配置文件示例
//! ```toml
//! [ai.providers.deepseek]
//! api_key = "${DEEPSEEK_API_KEY}"
//! model = "deepseek-chat"
//! base_url = "https://api.deepseek.com"
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
    DEEPSEEK_BASE_URL, DEEPSEEK_DEFAULT_MODEL,
    DEFAULT_TEMPERATURE, DEFAULT_MAX_TOKENS, DEFAULT_TIMEOUT,
    PROVIDER_DEEPSEEK,
};

/// DeepSeek Provider 配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeepSeekConfig {
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
}

impl Default for DeepSeekConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            base_url: None,
            model: Some(DEEPSEEK_DEFAULT_MODEL.to_string()),
            temperature: Some(DEFAULT_TEMPERATURE),
            max_tokens: Some(DEFAULT_MAX_TOKENS),
        }
    }
}

/// DeepSeek 聊天请求
#[derive(Debug, Serialize)]
struct DeepSeekChatRequest {
    /// 模型名称
    model: String,
    /// 消息列表
    messages: Vec<DeepSeekMessage>,
    /// 最大 Token 数
    max_tokens: Option<u32>,
    /// 温度参数
    temperature: Option<f32>,
    /// 是否流式响应
    pub stream: Option<bool>,
}

/// DeepSeek 消息
#[derive(Debug, Serialize, Deserialize)]
struct DeepSeekMessage {
    /// 角色
    role: String,
    /// 内容
    content: String,
}

/// DeepSeek 聊天响应
#[derive(Debug, Deserialize)]
struct DeepSeekChatResponse {
    /// 响应 ID
    id: String,
    /// 类型
    #[serde(rename = "object")]
    object_type: String,
    /// 创建时间戳
    created: u64,
    /// 模型
    model: String,
    /// 选择（回复内容）
    choices: Vec<DeepSeekChoice>,
    /// 使用统计
    usage: DeepSeekUsage,
}

/// DeepSeek 选择
#[derive(Debug, Deserialize)]
struct DeepSeekChoice {
    /// 索引
    index: u32,
    /// 消息
    message: DeepSeekMessage,
    /// 停止原因
    finish_reason: Option<String>,
}

/// DeepSeek 使用统计
#[derive(Debug, Deserialize)]
struct DeepSeekUsage {
    /// 提示 Token 数
    #[serde(rename = "prompt_tokens")]
    prompt_tokens: u32,
    /// 完成 Token 数
    #[serde(rename = "completion_tokens")]
    completion_tokens: u32,
    /// 总 Token 数
    #[serde(rename = "total_tokens")]
    total_tokens: u32,
}

/// DeepSeek Provider
///
/// 实现 DeepSeek 模型的 AI Provider
#[derive(Debug, Clone)]
pub struct DeepSeekProvider {
    /// 配置
    config: DeepSeekConfig,
    /// HTTP 客户端
    http_client: reqwest::Client,
}

impl DeepSeekProvider {
    /// 创建新的 DeepSeek Provider
    ///
    /// # 参数说明
    /// * `config` - Provider 配置
    ///
    /// # 返回值
    /// 创建的 Provider
    pub fn new(config: DeepSeekConfig) -> Self {
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
            .unwrap_or_else(|| DEEPSEEK_BASE_URL.to_string())
    }

    /// 获取模型名称
    fn get_model(&self) -> String {
        self.config.model.clone()
            .unwrap_or_else(|| DEEPSEEK_DEFAULT_MODEL.to_string())
    }
}

#[async_trait::async_trait]
impl AiProvider for DeepSeekProvider {
    /// 获取 Provider 名称
    fn name(&self) -> &str {
        PROVIDER_DEEPSEEK
    }

    /// 发送聊天请求
    async fn chat(&self, request: &ChatRequest) -> Result<ChatResponse> {
        let model = self.get_model();
        let base_url = self.get_base_url();

        debug!(model = %model, "发送 DeepSeek 聊天请求");

        // 构建 DeepSeek 格式的消息
        let messages: Vec<DeepSeekMessage> = request
            .messages
            .iter()
            .map(|msg| {
                let role = match msg.role {
                    super::MessageRole::User => "user",
                    super::MessageRole::Assistant => "assistant",
                    super::MessageRole::System => "system",
                    super::MessageRole::Tool => "user",
                    _ => "user",
                };

                DeepSeekMessage {
                    role: role.to_string(),
                    content: msg.content.clone(),
                }
            })
            .collect();

        // 构建请求
        let chat_request = DeepSeekChatRequest {
            model: model.clone(),
            messages,
            max_tokens: request.model.max_tokens.or(self.config.max_tokens),
            temperature: request.model.temperature.or(self.config.temperature),
            stream: Some(false),
        };

        // 构建请求头
        let request_builder = self.http_client
            .post(format!("{}/chat/completions", base_url))
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .header("Content-Type", "application/json");

        // 发送请求
        let response = request_builder
            .json(&chat_request)
            .send()
            .await
            .map_err(|e| crate::infra::error::Error::Ai(format!("DeepSeek API 请求失败: {}", e)))?;

        // 检查响应状态
        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            error!(status = ?status, error = %error_text, "DeepSeek API 错误");
            return Err(crate::infra::error::Error::Ai(format!("DeepSeek API 错误: {}", error_text)));
        }

        // 解析响应
        let response_body: DeepSeekChatResponse = response
            .json()
            .await
            .map_err(|e| crate::infra::error::Error::Ai(format!("解析 DeepSeek 响应失败: {}", e)))?;

        // 提取文本内容
        let content = response_body
            .choices
            .first()
            .map(|choice| choice.message.content.clone())
            .unwrap_or_default();

        let usage = TokenUsage {
            prompt_tokens: response_body.usage.prompt_tokens,
            completion_tokens: response_body.usage.completion_tokens,
            total_tokens: response_body.usage.total_tokens,
        };

        info!(
            model = %model,
            prompt_tokens = usage.prompt_tokens,
            completion_tokens = usage.completion_tokens,
            "DeepSeek 聊天响应"
        );

        Ok(ChatResponse {
            id: response_body.id,
            content,
            usage,
            done: true,
        })
    }

    /// 发送流式聊天请求
    async fn chat_stream(
        &self,
        request: &ChatRequest,
    ) -> Result<Box<dyn tokio::io::AsyncRead + Send + Unpin>> {
        let model = self.get_model();
        let base_url = self.get_base_url();

        debug!(model = %model, "发送 DeepSeek 流式聊天请求");

        // 构建 DeepSeek 格式的消息
        let messages: Vec<DeepSeekMessage> = request
            .messages
            .iter()
            .map(|msg| {
                let role = match msg.role {
                    super::MessageRole::User => "user",
                    super::MessageRole::Assistant => "assistant",
                    super::MessageRole::System => "system",
                    super::MessageRole::Tool => "user",
                    _ => "user",
                };

                DeepSeekMessage {
                    role: role.to_string(),
                    content: msg.content.clone(),
                }
            })
            .collect();

        // 构建请求
        let chat_request = DeepSeekChatRequest {
            model: model.clone(),
            messages,
            max_tokens: request.model.max_tokens.or(self.config.max_tokens),
            temperature: request.model.temperature.or(self.config.temperature),
            stream: Some(true),
        };

        // 构建请求头
        let request_builder = self.http_client
            .post(format!("{}/chat/completions", base_url))
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .header("Content-Type", "application/json");

        // 发送请求并获取流式响应
        let response = request_builder
            .json(&chat_request)
            .send()
            .await
            .map_err(|e| crate::infra::error::Error::Ai(format!("DeepSeek 流式请求失败: {}", e)))?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(crate::infra::error::Error::Ai(format!("DeepSeek API 错误: {}", error_text)));
        }

        // 将响应体转换为 AsyncRead
        // 使用 tokio::io::Empty 作为占位实现，实际应该实现 SSE 解析
        // 这里为了保持与 OpenAI 实现一致，暂时使用 empty
        Ok(Box::new(tokio::io::empty()) as Box<dyn tokio::io::AsyncRead + Send + Unpin>)
    }

    /// 创建嵌入向量
    async fn embeddings(&self, _texts: &[String], _model: &str) -> Result<Vec<Vec<f32>>> {
        // DeepSeek 目前可能不支持 embeddings 接口，或者接口与 OpenAI 一致
        // 暂时未实现
        Err(crate::infra::error::Error::Ai("DeepSeek 暂不支持 Embeddings".to_string()))
    }
}
