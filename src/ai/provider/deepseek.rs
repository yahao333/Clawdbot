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
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tracing::{debug, error, info, warn};

use super::{AiProvider, ChatMessage, ChatRequest, ChatResponse, ModelConfig, ProviderRegistry, TokenUsage};
use crate::infra::error::Result;
use crate::ai::constants::{
    DEEPSEEK_BASE_URL, DEEPSEEK_DEFAULT_MODEL,
    DEFAULT_TEMPERATURE, DEFAULT_MAX_TOKENS, DEFAULT_TIMEOUT,
    PROVIDER_DEEPSEEK,
};
use crate::ai::stream::{parse_sse_line, SseEvent};

/// DeepSeek SSE 响应流式读取器
///
/// 将 HTTP 字节流解析为 SSE 格式并提供 AsyncRead 接口
struct DeepSeekSseStream<S> {
    /// 内部字节流
    inner: S,
    /// 输出缓冲区
    output: Vec<u8>,
    /// 位置偏移
    position: usize,
}

impl<S> DeepSeekSseStream<S> {
    /// 创建新的 SSE 响应流读取器
    fn new(inner: S) -> Self {
        Self {
            inner,
            output: Vec::new(),
            position: 0,
        }
    }
}

impl<S: futures_util::Stream<Item = std::result::Result<bytes::Bytes, reqwest::Error>> + Unpin> DeepSeekSseStream<S> {
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

impl<S: futures_util::Stream<Item = std::result::Result<bytes::Bytes, reqwest::Error>> + Unpin> tokio::io::AsyncRead for DeepSeekSseStream<S> {
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
                // 递归尝试读取
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
            .header("Content-Type", "application/json")
            .header("Accept", "text/event-stream");

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

        // 创建 SSE 流式读取器
        let body = response.bytes_stream();
        let stream_reader = DeepSeekSseStream::new(body);

        debug!(model = %model, "开始流式响应");

        // 将其转换为 trait object
        let boxed_stream: Box<dyn tokio::io::AsyncRead + Send + Unpin> = Box::new(stream_reader);

        Ok(boxed_stream)
    }

    /// 创建嵌入向量
    async fn embeddings(&self, _texts: &[String], _model: &str) -> Result<Vec<Vec<f32>>> {
        // DeepSeek 目前可能不支持 embeddings 接口，或者接口与 OpenAI 一致
        // 暂时未实现
        Err(crate::infra::error::Error::Ai("DeepSeek 暂不支持 Embeddings".to_string()))
    }
}
