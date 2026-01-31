//! AI 引擎模块
//!
//! 负责协调 AI Provider、消息处理和响应生成。
//!
//! # 功能
//! 1. 管理 AI Provider 生命周期
//! 2. 加载 Provider 配置
//! 3. 执行 AI 推理
//! 4. 维护对话历史
//!
//! # 处理流程
//! ```
//! 1. 接收用户消息
//! 2. 获取对应 Agent 的 Provider
//! 3. 构建消息历史
//! 4. 调用 Provider 生成响应
//! 5. 返回响应内容
//! ```

use std::sync::Arc;
use tracing::{debug, error, info, instrument, warn};

use super::provider::{AiProvider, ChatMessage, ChatRequest, ModelConfig, ProviderRegistry, TokenUsage, MessageRole};
use crate::core::message::types::{InboundMessage, MessageContent};
use crate::infra::config::{AiConfig, Config, ProviderConfig};
use crate::infra::error::Result;

/// AI 引擎
///
/// 协调 AI Provider 执行推理任务
///
/// # 字段说明
/// * `registry` - Provider 注册表
/// * `config` - AI 配置
/// * `default_provider` - 默认 Provider 名称
/// * `conversation_history` - 对话历史（按用户/会话）
///
#[derive(Clone)]
pub struct AiEngine {
    /// Provider 注册表
    registry: Arc<ProviderRegistry>,
    /// AI 配置
    config: Arc<AiConfig>,
    /// 默认 Provider 名称
    default_provider: Option<String>,
    /// 对话历史缓存（用户 ID -> 消息历史）
    conversation_history: Arc<dashmap::DashMap<String, Vec<ChatMessage>>>,
    /// 最大历史消息数
    max_history: usize,
}

impl std::fmt::Debug for AiEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AiEngine")
            .field("registry", &self.registry)
            .field("config", &"AiConfig") // Config might contain secrets, so maybe just show placeholder or rely on its Debug impl
            .field("default_provider", &self.default_provider)
            .field("conversation_history_count", &self.conversation_history.len())
            .field("max_history", &self.max_history)
            .finish()
    }
}

impl AiEngine {
    /// 创建新的 AI 引擎
    ///
    /// # 参数说明
    /// * `config` - 主配置
    ///
    /// # 返回值
    /// 创建的 AI 引擎
    pub fn new(config: &Config) -> Self {
        let registry = Arc::new(ProviderRegistry::new());
        let ai_config = config.ai.clone();

        // 注册所有配置的 Provider
        Self::register_providers(&registry, &ai_config);

        Self {
            registry,
            config: Arc::new(ai_config),
            default_provider: None,
            conversation_history: Arc::new(dashmap::DashMap::new()),
            max_history: 20,
        }
    }

    /// 从 Provider 注册表创建 AI 引擎
    ///
    /// 用于测试或手动注册 Provider 的场景
    ///
    /// # 参数说明
    /// * `registry` - 已配置的 Provider 注册表
    /// * `config` - AI 配置
    ///
    /// # 返回值
    /// 创建的 AI 引擎
    pub fn with_registry(registry: Arc<ProviderRegistry>, config: Arc<AiConfig>) -> Self {
        Self {
            registry,
            config,
            default_provider: None,
            conversation_history: Arc::new(dashmap::DashMap::new()),
            max_history: 20,
        }
    }

    /// 注册所有配置的 Provider
    fn register_providers(registry: &ProviderRegistry, ai_config: &AiConfig) {
        for (name, provider_config) in &ai_config.providers {
            debug!(provider = %name, "注册 Provider");

            match name.as_str() {
                "anthropic" => {
                    let config = super::provider::anthropic::AnthropicConfig {
                        api_key: provider_config.api_key.clone().unwrap_or_default(),
                        base_url: provider_config.base_url.clone(),
                        model: provider_config.model.clone(),
                        temperature: provider_config.temperature,
                        max_tokens: provider_config.max_tokens,
                        api_version: Some("2023-06-01".to_string()),
                    };
                    let provider = super::provider::anthropic::AnthropicProvider::new(config);
                    registry.register(provider);
                    info!(provider = %name, "Anthropic Provider 注册成功");
                }
                "openai" => {
                    let config = super::provider::openai::OpenAIConfig {
                        api_key: provider_config.api_key.clone().unwrap_or_default(),
                        base_url: provider_config.base_url.clone(),
                        organization_id: None,
                        model: provider_config.model.clone(),
                        temperature: provider_config.temperature,
                        max_tokens: provider_config.max_tokens,
                    };
                    let provider = super::provider::openai::OpenAIProvider::new(config);
                    registry.register(provider);
                    info!(provider = %name, "OpenAI Provider 注册成功");
                }
                "minimax" => {
                    let config = super::provider::minimax::MiniMaxConfig {
                        api_key: provider_config.api_key.clone().unwrap_or_default(),
                        group_id: provider_config.group_id.clone().unwrap_or_default(),
                        model: provider_config.model.clone(),
                        temperature: provider_config.temperature,
                        max_tokens: provider_config.max_tokens,
                        base_url: None,
                    };
                    let provider = super::provider::minimax::MiniMaxProvider::new(config);
                    registry.register(provider);
                    info!(provider = %name, "MiniMax Provider 注册成功");
                }
                "deepseek" => {
                    let config = super::provider::deepseek::DeepSeekConfig {
                        api_key: provider_config.api_key.clone().unwrap_or_default(),
                        base_url: provider_config.base_url.clone(),
                        model: provider_config.model.clone(),
                        temperature: provider_config.temperature,
                        max_tokens: provider_config.max_tokens,
                    };
                    let provider = super::provider::deepseek::DeepSeekProvider::new(config);
                    registry.register(provider);
                    info!(provider = %name, "DeepSeek Provider 注册成功");
                }
                _ => {
                    warn!(provider = %name, "未支持的 Provider 类型");
                }
            }
        }
    }

    /// 获取 Provider
    ///
    /// 根据名称或默认配置获取 Provider
    ///
    /// # 参数说明
    /// * `name` - Provider 名称
    ///
    /// # 返回值
    /// Provider 实例
    pub fn get_provider(&self, name: Option<&str>) -> Option<Arc<dyn AiProvider>> {
        let provider_name = name.or(self.default_provider.as_deref())
            .or(self.config.default_provider.as_deref());

        if let Some(name) = provider_name {
            self.registry.get(name)
        } else {
            // 返回第一个可用的 Provider
            self.registry.list().first()
                .and_then(|name| self.registry.get(name))
        }
    }

    /// 执行 Agent 生成响应
    ///
    /// # 参数说明
    /// * `agent_id` - Agent ID（用于确定使用哪个 Provider）
    /// * `message` - 输入消息
    ///
    /// # 返回值
    /// 生成的响应内容
    #[instrument(skip(self, message), fields(agent_id))]
    pub async fn execute(
        &self,
        agent_id: &str,
        message: &InboundMessage,
    ) -> Result<MessageContent> {
        debug!(agent_id = agent_id, "开始执行 AI 推理");

        // 1. 确定使用的 Provider
        let provider_name = self.get_agent_provider(agent_id);
        let provider = self.get_provider(Some(&provider_name))
            .ok_or_else(|| crate::infra::error::Error::Ai(format!("Provider 未找到: {}", provider_name)))?;

        debug!(provider = %provider.name(), "使用 Provider");

        // 2. 构建消息
        let messages = self.build_messages(agent_id, message).await?;

        // 3. 获取 Agent 配置
        let model_config = self.get_agent_model_config(agent_id);

        // 4. 构建请求
        let request = ChatRequest {
            model: model_config,
            messages,
            tools: Vec::new(),
            stream: false,
        };

        // 5. 调用 Provider
        let response = provider.chat(&request).await
            .map_err(|e| crate::infra::error::Error::Ai(format!("AI 调用失败: {}", e)))?;

        // 6. 更新对话历史
        self.update_history(agent_id, message, &response.content).await;

        info!(
            agent_id = agent_id,
            provider = %provider.name(),
            prompt_tokens = response.usage.prompt_tokens,
            completion_tokens = response.usage.completion_tokens,
            "AI 推理完成"
        );

        Ok(MessageContent::text(&response.content))
    }

    /// 执行流式响应
    ///
    /// # 参数说明
    /// * `agent_id` - Agent ID
    /// * `message` - 输入消息
    ///
    /// # 返回值
    /// 流式响应读取器
    pub async fn execute_stream(
        &self,
        agent_id: &str,
        message: &InboundMessage,
    ) -> Result<Box<dyn tokio::io::AsyncRead + Send + Unpin>> {
        let provider_name = self.get_agent_provider(agent_id);
        let provider = self.get_provider(Some(&provider_name))
            .ok_or_else(|| crate::infra::error::Error::Ai(format!("Provider 未找到: {}", provider_name)))?;

        let messages = self.build_messages(agent_id, message).await?;
        let model_config = self.get_agent_model_config(agent_id);

        let request = ChatRequest {
            model: model_config,
            messages,
            tools: Vec::new(),
            stream: true,
        };

        provider.chat_stream(&request).await
            .map_err(|e| crate::infra::error::Error::Ai(format!("流式调用失败: {}", e)))
    }

    /// 获取 Agent 使用的 Provider 名称
    fn get_agent_provider(&self, agent_id: &str) -> String {
        // TODO: 从 Agent 配置获取 Provider
        // 临时使用默认 Provider
        self.config.default_provider.clone()
            .unwrap_or_else(|| {
                self.registry.list().first()
                    .cloned()
                    .unwrap_or_else(|| "openai".to_string())
            })
    }

    /// 获取 Agent 的模型配置
    fn get_agent_model_config(&self, agent_id: &str) -> ModelConfig {
        // TODO: 从 Agent 配置获取模型配置
        // 临时使用默认配置
        let provider_name = self.get_agent_provider(agent_id);

        ModelConfig {
            provider: provider_name.clone(),
            model: self.config.providers.get(&provider_name)
                .and_then(|c| c.model.clone())
                .unwrap_or_else(|| "gpt-4o".to_string()),
            api_key: None,
            base_url: None,
            temperature: self.config.providers.get(&provider_name)
                .and_then(|c| c.temperature),
            max_tokens: self.config.providers.get(&provider_name)
                .and_then(|c| c.max_tokens),
            system_prompt: None,
        }
    }

    /// 构建消息列表
    ///
    /// 包含系统提示和对话历史
    async fn build_messages(
        &self,
        agent_id: &str,
        message: &InboundMessage,
    ) -> Result<Vec<ChatMessage>> {
        let mut messages: Vec<ChatMessage> = Vec::new();

        // 添加系统消息（如果有）
        // TODO: 从 Agent 配置获取系统提示
        messages.push(ChatMessage {
            role: MessageRole::System,
            content: "你是一个智能助手，请用简洁清晰的语言回答用户的问题。".to_string(),
            name: None,
        });

        // 获取历史消息
        let history = self.get_history(agent_id).await;

        // 添加历史消息（限制数量）
        let max_history = self.max_history.saturating_sub(1); // 减去系统消息
        for msg in history.into_iter().rev().take(max_history) {
            messages.push(msg);
        }

        // 添加当前用户消息
        let text = message.content.text.clone()
            .or(message.content.rich_text.as_ref().map(|rt| rt.content.clone()))
            .unwrap_or_else(String::new);

        messages.push(ChatMessage {
            role: MessageRole::User,
            content: text.to_string(),
            name: None,
        });

        Ok(messages)
    }

    /// 获取对话历史
    async fn get_history(&self, agent_id: &str) -> Vec<ChatMessage> {
        self.conversation_history
            .get(agent_id)
            .map(|h| h.clone())
            .unwrap_or_default()
    }

    /// 更新对话历史
    async fn update_history(&self, agent_id: &str, message: &InboundMessage, response: &str) {
        let text = message.content.text.clone()
            .or(message.content.rich_text.as_ref().map(|rt| rt.content.clone()))
            .unwrap_or_else(String::new);

        let mut history = self.get_history(agent_id).await;

        // 添加用户消息
        history.push(ChatMessage {
            role: MessageRole::User,
            content: text.to_string(),
            name: None,
        });

        // 添加助手响应
        history.push(ChatMessage {
            role: MessageRole::Assistant,
            content: response.to_string(),
            name: None,
        });

        // 限制历史长度
        if history.len() > self.max_history {
            let skip_count = history.len() - self.max_history;
            let new_history: Vec<ChatMessage> = history.iter()
                .skip(skip_count)
                .cloned()
                .collect();
            history = new_history;
        }

        // 更新缓存
        let mut entry = self.conversation_history.entry(agent_id.to_string()).or_default();
        *entry = history;
    }

    /// 清空对话历史
    ///
    /// # 参数说明
    /// * `agent_id` - 要清空的 Agent ID，None 表示清空所有
    pub async fn clear_history(&self, agent_id: Option<&str>) {
        if let Some(id) = agent_id {
            self.conversation_history.remove(id);
        } else {
            self.conversation_history.clear();
        }
        info!(agent_id = agent_id.map(|s| s.to_string()).unwrap_or_else(|| "all".to_string()), "对话历史已清空");
    }

    /// 列出所有可用的 Provider
    pub fn list_providers(&self) -> Vec<String> {
        self.registry.list()
    }

    /// 检查 Provider 是否可用
    pub fn has_provider(&self, name: &str) -> bool {
        self.registry.contains(name)
    }
}

impl Default for AiEngine {
    fn default() -> Self {
        Self::new(&Config::default())
    }
}
