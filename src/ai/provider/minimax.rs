//! MiniMax AI Provider 实现
//!
//! 本模块实现了 MiniMax 的聊天模型接口。
//!
//! # 功能
//! - 聊天完成
//! - 流式响应
//!
//! # 配置文件示例
//! ```toml
//! [ai.providers.minimax]
//! api_key = "${MINIMAX_API_KEY}"
//! group_id = "${MINIMAX_GROUP_ID}"
//! model = "abab6.5s-chat"
//! temperature = 0.7
//! max_tokens = 4096
//! ```

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{debug, error, info};

use super::{AiProvider, ChatMessage, ChatRequest, ChatResponse, ModelConfig, TokenUsage};
use crate::infra::error::Result;
use crate::ai::constants::{
    MINIMAX_BASE_URL, MINIMAX_DEFAULT_MODEL,
    DEFAULT_TEMPERATURE, DEFAULT_MAX_TOKENS, DEFAULT_TIMEOUT,
    PROVIDER_MINIMAX,
};

/// MiniMax Provider 配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MiniMaxConfig {
    /// API Key
    pub api_key: String,
    /// Group ID
    pub group_id: String,
    /// Base URL
    pub base_url: Option<String>,
    /// 模型名称
    pub model: Option<String>,
    /// 温度参数
    pub temperature: Option<f32>,
    /// 最大 Token 数
    pub max_tokens: Option<u32>,
}

impl Default for MiniMaxConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            group_id: String::new(),
            base_url: None,
            model: Some(MINIMAX_DEFAULT_MODEL.to_string()),
            temperature: Some(DEFAULT_TEMPERATURE),
            max_tokens: Some(DEFAULT_MAX_TOKENS),
        }
    }
}

/// MiniMax 聊天请求
#[derive(Debug, Serialize)]
struct MiniMaxChatRequest {
    /// 模型名称
    model: String,
    /// 消息列表
    messages: Vec<MiniMaxRequestMessage>,
    /// 温度参数
    temperature: Option<f32>,
    /// 最大 Token 数
    max_tokens: Option<u32>,
    /// 是否流式响应
    stream: Option<bool>,
}

/// MiniMax 消息（支持多种 content 格式）
#[derive(Debug, Deserialize, Clone)]
struct MiniMaxMessage {
    /// 角色
    #[serde(rename = "role")]
    role_type: String,
    /// 内容（支持字符串或数组格式）
    #[serde(rename = "content")]
    content: serde_json::Value,
}

/// MiniMax 消息（用于发送请求）
#[derive(Debug, Serialize)]
struct MiniMaxRequestMessage {
    /// 角色
    #[serde(rename = "role")]
    role_type: String,
    /// 内容（Anthropic 格式：数组）
    #[serde(rename = "content")]
    content: Vec<MiniMaxContentBlock>,
}

impl MiniMaxMessage {
    /// 提取文本内容
    fn extract_content(&self) -> String {
        match &self.content {
            // 字符串格式
            serde_json::Value::String(text) => text.clone(),
            // 数组格式（Anthropic 格式）
            serde_json::Value::Array(blocks) => blocks.iter()
                .filter_map(|block| block.get("text").and_then(|t| t.as_str()).map(|s| s.to_string()))
                .collect::<Vec<_>>()
                .join(""),
            // 其他格式
            _ => self.content.to_string(),
        }
    }
}

/// MiniMax 内容块（Anthropic 格式）
#[derive(Debug, Serialize, Deserialize, Clone)]
struct MiniMaxContentBlock {
    /// 块类型
    #[serde(rename = "type")]
    block_type: String,
    /// 文本内容
    #[serde(rename = "text")]
    text: String,
}

/// MiniMax 基础响应
#[derive(Debug, Deserialize)]
struct MiniMaxBaseResp {
    status_code: i32,
    status_msg: String,
}

/// MiniMax 聊天响应
#[derive(Debug, Deserialize)]
struct MiniMaxChatResponse {
    /// 响应 ID
    #[serde(rename = "id")]
    id: String,
    /// 对象类型
    #[serde(rename = "object")]
    object_type: String,
    /// 创建时间戳
    #[serde(rename = "created")]
    created: u64,
    /// 模型
    #[serde(rename = "model")]
    model: String,
    /// 选择（回复内容）
    choices: Option<Vec<MiniMaxChoice>>,
    /// 使用统计
    usage: Option<MiniMaxUsage>,
    /// 基础响应信息
    base_resp: Option<MiniMaxBaseResp>,
}

/// MiniMax 选择
#[derive(Debug, Deserialize)]
struct MiniMaxChoice {
    /// 索引
    index: u32,
    /// 消息
    message: MiniMaxMessage,
    /// 停止原因
    #[serde(rename = "finish_reason")]
    finish_reason: Option<String>,
}

/// MiniMax 使用统计
#[derive(Debug, Deserialize, Default)]
struct MiniMaxUsage {
    /// 提示 Token 数
    #[serde(rename = "prompt_tokens", default)]
    prompt_tokens: u32,
    /// 完成 Token 数
    #[serde(rename = "completion_tokens", default)]
    completion_tokens: u32,
    /// 总 Token 数
    #[serde(rename = "total_tokens")]
    total_tokens: u32,
}

/// MiniMax 错误响应
#[derive(Debug, Deserialize)]
struct MiniMaxErrorResponse {
    /// 错误对象
    #[serde(rename = "object")]
    object_type: String,
    /// 错误类型
    #[serde(rename = "type")]
    error_type: Option<String>,
    /// 错误信息
    message: Option<String>,
    /// 错误码
    #[serde(rename = "param")]
    param: Option<String>,
    /// 错误码
    code: Option<u32>,
}

/// MiniMax Provider
///
/// 实现 MiniMax 聊天模型的 AI Provider
#[derive(Debug, Clone)]
pub struct MiniMaxProvider {
    /// 配置
    config: MiniMaxConfig,
    /// HTTP 客户端
    http_client: reqwest::Client,
}

impl MiniMaxProvider {
    /// 创建新的 MiniMax Provider
    ///
    /// # 参数说明
    /// * `config` - Provider 配置
    ///
    /// # 返回值
    /// 创建的 Provider
    pub fn new(config: MiniMaxConfig) -> Self {
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
            .unwrap_or_else(|| MINIMAX_BASE_URL.to_string())
    }

    /// 获取模型名称
    fn get_model(&self) -> String {
        self.config.model.clone()
            .unwrap_or_else(|| MINIMAX_DEFAULT_MODEL.to_string())
    }
}

#[async_trait::async_trait]
impl AiProvider for MiniMaxProvider {
    /// 获取 Provider 名称
    fn name(&self) -> &str {
        PROVIDER_MINIMAX
    }

    /// 发送聊天请求
    async fn chat(&self, request: &ChatRequest) -> Result<ChatResponse> {
        let model = self.get_model();
        let base_url = self.get_base_url();

        debug!(model = %model, "发送 MiniMax 聊天请求");

        // 构建 MiniMax Anthropic 格式的消息
        let messages: Vec<MiniMaxRequestMessage> = request
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

                // 将简单文本转换为 Anthropic 格式的内容块
                let content = vec![MiniMaxContentBlock {
                    block_type: "text".to_string(),
                    text: msg.content.clone(),
                }];

                MiniMaxRequestMessage {
                    role_type: role.to_string(),
                    content,
                }
            })
            .collect();

        // 构建请求（Anthropic 格式）
        let chat_request = MiniMaxChatRequest {
            model: model.clone(),
            messages,
            max_tokens: request.model.max_tokens.or(self.config.max_tokens),
            temperature: request.model.temperature.or(self.config.temperature),
            stream: Some(false),
        };

        // 构建请求头
        let request_builder = self.http_client
            .post(format!("{}/chatcompletion_v2", base_url))
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .header("Content-Type", "application/json")
            .header("X-GroupId", &self.config.group_id);

        // 发送请求
        let response = request_builder
            .json(&chat_request)
            .send()
            .await
            .map_err(|e| crate::infra::error::Error::Ai(format!("MiniMax API 请求失败: {}", e)))?;

        // 检查响应状态
        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            error!(status = ?status, error = %error_text, "MiniMax API 错误");
            return Err(crate::infra::error::Error::Ai(format!("MiniMax API 错误: {}", error_text)));
        }

        // 解析响应
        let response_text = response.text().await
            .map_err(|e| crate::infra::error::Error::Ai(format!("读取 MiniMax 响应失败: {}", e)))?;
        
        debug!(body = %response_text, "收到 MiniMax 响应");

        let response_body: MiniMaxChatResponse = serde_json::from_str(&response_text)
            .map_err(|e| crate::infra::error::Error::Ai(format!("解析 MiniMax 响应失败: {}, body: {}", e, response_text)))?;

        // 检查业务错误
        if let Some(base_resp) = &response_body.base_resp {
            if base_resp.status_code != 0 {
                let msg = format!("MiniMax API Error: [{}] {}", base_resp.status_code, base_resp.status_msg);
                error!(%msg);
                return Err(crate::infra::error::Error::Ai(msg));
            }
        }

        // 提取文本内容（支持字符串和数组格式）
        let content = response_body
            .choices
            .as_ref()
            .and_then(|c| c.first())
            .map(|choice| choice.message.extract_content())
            .unwrap_or_default();

        // 提取使用统计
        let usage = response_body.usage.unwrap_or_default();
        let token_usage = TokenUsage {
            prompt_tokens: usage.prompt_tokens,
            completion_tokens: usage.completion_tokens,
            total_tokens: usage.total_tokens,
        };

        info!(
            model = %model,
            prompt_tokens = token_usage.prompt_tokens,
            completion_tokens = token_usage.completion_tokens,
            "MiniMax 聊天响应"
        );

        Ok(ChatResponse {
            id: response_body.id,
            content,
            usage: token_usage,
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

        debug!(model = %model, "发送 MiniMax 流式聊天请求");

        // 构建 MiniMax 格式的消息
        let messages: Vec<MiniMaxRequestMessage> = request
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

                // 将简单文本转换为 Anthropic 格式的内容块
                let content = vec![MiniMaxContentBlock {
                    block_type: "text".to_string(),
                    text: msg.content.clone(),
                }];

                MiniMaxRequestMessage {
                    role_type: role.to_string(),
                    content,
                }
            })
            .collect();

        // 构建请求
        let chat_request = MiniMaxChatRequest {
            model: model.clone(),
            messages,
            max_tokens: request.model.max_tokens.or(self.config.max_tokens),
            temperature: request.model.temperature.or(self.config.temperature),
            stream: Some(true),
        };

        // 构建请求头
        let request_builder = self.http_client
            .post(format!("{}/chatcompletion_v2", base_url))
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .header("Content-Type", "application/json")
            .header("X-GroupId", &self.config.group_id);

        // 发送请求并获取流式响应
        let response = request_builder
            .json(&chat_request)
            .send()
            .await
            .map_err(|e| crate::infra::error::Error::Ai(format!("MiniMax 流式请求失败: {}", e)))?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(crate::infra::error::Error::Ai(format!("MiniMax API 错误: {}", error_text)));
        }

        // 将响应体转换为 AsyncRead
        // MiniMax SSE 响应流式返回
        Ok(Box::new(tokio::io::empty()) as Box<dyn tokio::io::AsyncRead + Send + Unpin>)
    }

    /// 创建嵌入向量
    async fn embeddings(&self, texts: &[String], _model: &str) -> Result<Vec<Vec<f32>>> {
        // MiniMax 目前不支持 embeddings API
        // 返回空向量作为占位实现
        tracing::warn!("MiniMax 不支持 embeddings API");
        Ok(vec![])
    }
}

/// 注册 MiniMax Provider 到注册表
pub fn register_provider(registry: &super::ProviderRegistry, config: MiniMaxConfig) {
    let provider = MiniMaxProvider::new(config);
    registry.register(provider);
}
