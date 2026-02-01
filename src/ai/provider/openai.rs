//! OpenAI AI Provider 实现
//!
//! 本模块实现了 OpenAI 的 GPT 模型接口。
//!
//! # 功能
//! - 聊天完成
//! - 流式响应
//! - 嵌入向量
//!
//! # 配置文件示例
//! ```toml
//! [ai.providers.openai]
//! api_key = "${OPENAI_API_KEY}"
//! model = "gpt-4o"
//! base_url = "https://api.openai.com/v1"
//! temperature = 0.7
//! max_tokens = 4096
//! ```

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;
use tracing::{debug, error, info, warn};

use super::{AiProvider, ChatMessage, ChatRequest, ChatResponse, ModelConfig, ProviderRegistry, TokenUsage};
use crate::infra::error::Result;
use crate::ai::constants::{
    OPENAI_BASE_URL, OPENAI_DEFAULT_MODEL,
    DEFAULT_TEMPERATURE, DEFAULT_MAX_TOKENS, DEFAULT_TIMEOUT,
    POOL_IDLE_TIMEOUT, POOL_MAX_IDLE_PER_HOST,
};
use crate::ai::stream::{parse_sse_line, SseEvent};

/// SSE 响应流式读取器
///
/// 将 HTTP 字节流解析为 SSE 格式并提供 AsyncRead 接口
struct SseResponseStream<S> {
    /// 内部字节流
    inner: S,
    /// 输出缓冲区
    output: Vec<u8>,
    /// 位置偏移
    position: usize,
}

impl<S> SseResponseStream<S> {
    /// 创建新的 SSE 响应流读取器
    fn new(inner: S) -> Self {
        Self {
            inner,
            output: Vec::new(),
            position: 0,
        }
    }
}

impl<S: futures_util::Stream<Item = std::result::Result<bytes::Bytes, reqwest::Error>> + Unpin> SseResponseStream<S> {
    /// 处理内部流的一个 chunk
    fn process_chunk(&mut self, chunk: bytes::Bytes) {
        let chunk_str = match std::str::from_utf8(&chunk) {
            Ok(s) => s,
            Err(_) => {
                self.output.extend_from_slice(&chunk);
                return;
            }
        };

        for line in chunk_str.lines() {
            match parse_sse_line(line) {
                SseEvent::Data(content) => {
                    if !content.is_empty() {
                        self.output.extend_from_slice(content.as_bytes());
                    }
                }
                SseEvent::Done => {
                    self.output.extend_from_slice(b"[DONE]");
                }
                SseEvent::Error(msg) => {
                    let error_msg = format!("ERROR: {}\n", msg);
                    self.output.extend_from_slice(error_msg.as_bytes());
                }
                SseEvent::Ignore => {}
            }
        }
    }
}

impl<S: futures_util::Stream<Item = std::result::Result<bytes::Bytes, reqwest::Error>> + Unpin> tokio::io::AsyncRead for SseResponseStream<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        // 如果输出缓冲区有数据，直接返回
        if self.position < self.output.len() {
            let remaining = &self.output[self.position..];
            let to_read = std::cmp::min(remaining.len(), buf.remaining());

            if to_read > 0 {
                buf.put_slice(&remaining[..to_read]);
                self.position += to_read;
            }
            return Poll::Ready(Ok(()));
        }

        // 重置缓冲区
        self.output.clear();
        self.position = 0;

        // 尝试从内部流读取数据
        match Pin::new(&mut self.inner).poll_next(cx) {
            Poll::Ready(Some(Ok(chunk))) => {
                self.process_chunk(chunk);
                // 递归尝试读取（尾递归优化）
                self.poll_read(cx, buf)
            }
            Poll::Ready(Some(Err(e))) => {
                let error_msg = format!("Stream error: {}", e);
                buf.put_slice(error_msg.as_bytes());
                Poll::Ready(Ok(()))
            }
            Poll::Ready(None) => {
                // 流结束
                Poll::Ready(Ok(()))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

/// OpenAI Provider 配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAIConfig {
    /// API Key
    pub api_key: String,
    /// API Base URL
    pub base_url: Option<String>,
    /// 组织 ID（可选）
    pub organization_id: Option<String>,
    /// 模型名称
    pub model: Option<String>,
    /// 温度参数
    pub temperature: Option<f32>,
    /// 最大 Token 数
    pub max_tokens: Option<u32>,
}

impl Default for OpenAIConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            base_url: None,
            organization_id: None,
            model: Some(OPENAI_DEFAULT_MODEL.to_string()),
            temperature: Some(DEFAULT_TEMPERATURE),
            max_tokens: Some(DEFAULT_MAX_TOKENS),
        }
    }
}

/// OpenAI 聊天请求
#[derive(Debug, Serialize)]
struct OpenAIChatRequest {
    /// 模型名称
    model: String,
    /// 消息列表
    messages: Vec<OpenAIMessage>,
    /// 最大 Token 数
    max_tokens: Option<u32>,
    /// 温度参数
    temperature: Option<f32>,
    /// 是否流式响应
    pub stream: Option<bool>,
}

/// OpenAI 消息
#[derive(Debug, Serialize, Deserialize)]
struct OpenAIMessage {
    /// 角色
    role: String,
    /// 内容
    content: String,
}

/// OpenAI 聊天响应
#[derive(Debug, Deserialize)]
struct OpenAIChatResponse {
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
    choices: Vec<OpenAIChoice>,
    /// 使用统计
    usage: OpenAIUsage,
}

/// OpenAI 选择
#[derive(Debug, Deserialize)]
struct OpenAIChoice {
    /// 索引
    index: u32,
    /// 消息
    message: OpenAIMessage,
    /// 停止原因
    finish_reason: Option<String>,
}

/// OpenAI 使用统计
#[derive(Debug, Deserialize)]
struct OpenAIUsage {
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

/// OpenAI Provider
///
/// 实现 OpenAI GPT 模型的 AI Provider
#[derive(Debug, Clone)]
pub struct OpenAIProvider {
    /// 配置
    config: OpenAIConfig,
    /// HTTP 客户端
    http_client: reqwest::Client,
}

impl OpenAIProvider {
    /// 创建新的 OpenAI Provider
    ///
    /// # 参数说明
    /// * `config` - Provider 配置
    ///
    /// # 返回值
    /// 创建的 Provider
    pub fn new(config: OpenAIConfig) -> Self {
        let http_client = reqwest::Client::builder()
            .timeout(DEFAULT_TIMEOUT)
            .pool_idle_timeout(POOL_IDLE_TIMEOUT)
            .pool_max_idle_per_host(POOL_MAX_IDLE_PER_HOST)
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
            .unwrap_or_else(|| OPENAI_BASE_URL.to_string())
    }

    /// 获取模型名称
    fn get_model(&self) -> String {
        self.config.model.clone()
            .unwrap_or_else(|| OPENAI_DEFAULT_MODEL.to_string())
    }
}

#[async_trait::async_trait]
impl AiProvider for OpenAIProvider {
    /// 获取 Provider 名称
    fn name(&self) -> &str {
        "openai"
    }

    /// 发送聊天请求
    async fn chat(&self, request: &ChatRequest) -> Result<ChatResponse> {
        let model = self.get_model();
        let base_url = self.get_base_url();

        debug!(model = %model, "发送 OpenAI 聊天请求");

        // 构建 OpenAI 格式的消息
        let messages: Vec<OpenAIMessage> = request
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

                OpenAIMessage {
                    role: role.to_string(),
                    content: msg.content.clone(),
                }
            })
            .collect();

        // 构建请求
        let chat_request = OpenAIChatRequest {
            model: model.clone(),
            messages,
            max_tokens: request.model.max_tokens.or(self.config.max_tokens),
            temperature: request.model.temperature.or(self.config.temperature),
            stream: Some(false),
        };

        // 构建请求头
        let mut request_builder = self.http_client
            .post(format!("{}/chat/completions", base_url))
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .header("Content-Type", "application/json");

        // 添加组织 ID（如果有）
        if let Some(org_id) = &self.config.organization_id {
            request_builder = request_builder.header("OpenAI-Organization", org_id);
        }

        // 发送请求
        let response = request_builder
            .json(&chat_request)
            .send()
            .await
            .map_err(|e| crate::infra::error::Error::Ai(format!("OpenAI API 请求失败: {}", e)))?;

        // 检查响应状态
        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            error!(status = ?status, error = %error_text, "OpenAI API 错误");
            return Err(crate::infra::error::Error::Ai(format!("OpenAI API 错误: {}", error_text)));
        }

        // 解析响应
        let response_body: OpenAIChatResponse = response
            .json()
            .await
            .map_err(|e| crate::infra::error::Error::Ai(format!("解析 OpenAI 响应失败: {}", e)))?;

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
            "OpenAI 聊天响应"
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

        debug!(model = %model, "发送 OpenAI 流式聊天请求");

        // 构建 OpenAI 格式的消息
        let messages: Vec<OpenAIMessage> = request
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

                OpenAIMessage {
                    role: role.to_string(),
                    content: msg.content.clone(),
                }
            })
            .collect();

        // 构建请求
        let chat_request = OpenAIChatRequest {
            model: model.clone(),
            messages,
            max_tokens: request.model.max_tokens.or(self.config.max_tokens),
            temperature: request.model.temperature.or(self.config.temperature),
            stream: Some(true),
        };

        // 构建请求头
        let mut request_builder = self.http_client
            .post(format!("{}/chat/completions", base_url))
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .header("Content-Type", "application/json")
            .header("Accept", "text/event-stream");

        if let Some(org_id) = &self.config.organization_id {
            request_builder = request_builder.header("OpenAI-Organization", org_id);
        }

        // 发送请求并获取流式响应
        let response = request_builder
            .json(&chat_request)
            .send()
            .await
            .map_err(|e| crate::infra::error::Error::Ai(format!("OpenAI 流式请求失败: {}", e)))?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(crate::infra::error::Error::Ai(format!("OpenAI API 错误: {}", error_text)));
        }

        // 创建 SSE 流式读取器
        let body = response.bytes_stream();
        let stream_reader = SseResponseStream::new(body);

        debug!(model = %model, "开始流式响应");

        // 将其转换为 trait object
        let boxed_stream: Box<dyn tokio::io::AsyncRead + Send + Unpin> = Box::new(stream_reader);

        Ok(boxed_stream)
    }

    /// 创建嵌入向量
    async fn embeddings(&self, texts: &[String], model: &str) -> Result<Vec<Vec<f32>>> {
        let base_url = self.get_base_url();
        let model = model.to_string();

        debug!(model = %model, text_count = texts.len(), "创建 OpenAI 嵌入向量");

        #[derive(Serialize)]
        struct EmbeddingsRequest {
            model: String,
            input: Vec<String>,
        }

        #[derive(Deserialize)]
        struct EmbeddingsResponse {
            data: Vec<EmbeddingData>,
            usage: OpenAIUsage,
        }

        #[derive(Deserialize)]
        struct EmbeddingData {
            embedding: Vec<f32>,
        }

        let request_body = EmbeddingsRequest {
            model,
            input: texts.to_vec(),
        };

        let response = self.http_client
            .post(format!("{}/embeddings", base_url))
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await
            .map_err(|e| crate::infra::error::Error::Ai(format!("OpenAI 嵌入请求失败: {}", e)))?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(crate::infra::error::Error::Ai(format!("OpenAI 嵌入 API 错误: {}", error_text)));
        }

        let response_body: EmbeddingsResponse = response
            .json()
            .await
            .map_err(|e| crate::infra::error::Error::Ai(format!("解析 OpenAI 嵌入响应失败: {}", e)))?;

        let embeddings: Vec<Vec<f32>> = response_body
            .data
            .into_iter()
            .map(|d| d.embedding)
            .collect();

        info!(embedding_count = embeddings.len(), "OpenAI 嵌入向量创建成功");

        Ok(embeddings)
    }
}

/// 注册 OpenAI Provider 到注册表
pub fn register_provider(registry: &ProviderRegistry, config: OpenAIConfig) {
    let provider = OpenAIProvider::new(config);
    registry.register(provider);
}
