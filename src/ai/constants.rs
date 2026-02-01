//! AI 模块常量定义
//!
//! 集中管理 AI 模块中的所有硬编码常量，避免多处重复定义
//!
//! # 常量分类
//! - Provider 名称
//! - 默认模型
//! - API 基础 URL
//! - 默认参数
//! - 系统提示词

use std::time::Duration;

/// ==================== Provider 名称 ====================

/// Anthropic Provider 名称
pub const PROVIDER_ANTHROPIC: &str = "anthropic";

/// OpenAI Provider 名称
pub const PROVIDER_OPENAI: &str = "openai";

/// DeepSeek Provider 名称
pub const PROVIDER_DEEPSEEK: &str = "deepseek";

/// MiniMax Provider 名称
pub const PROVIDER_MINIMAX: &str = "minimax";

/// 默认 Provider 名称（当配置未指定时使用）
pub const DEFAULT_PROVIDER: &str = PROVIDER_OPENAI;

/// ==================== 默认模型 ====================

/// Anthropic 默认模型
pub const ANTHROPIC_DEFAULT_MODEL: &str = "claude-3-5-sonnet-20241022";

/// OpenAI 默认模型
pub const OPENAI_DEFAULT_MODEL: &str = "gpt-4o";

/// DeepSeek 默认模型
pub const DEEPSEEK_DEFAULT_MODEL: &str = "deepseek-chat";

/// MiniMax 默认模型
pub const MINIMAX_DEFAULT_MODEL: &str = "abab6.5s-chat";

/// ==================== API 基础 URL ====================

/// OpenAI API 基础 URL
pub const OPENAI_BASE_URL: &str = "https://api.openai.com/v1";

/// Anthropic API 基础 URL
pub const ANTHROPIC_BASE_URL: &str = "https://api.anthropic.com/v1";

/// DeepSeek API 基础 URL
pub const DEEPSEEK_BASE_URL: &str = "https://api.deepseek.com";

/// MiniMax API 基础 URL
pub const MINIMAX_BASE_URL: &str = "https://api.minimaxi.com/v1";

/// ==================== 默认参数 ====================

/// 默认温度参数
pub const DEFAULT_TEMPERATURE: f32 = 0.7;

/// 默认最大 Token 数
pub const DEFAULT_MAX_TOKENS: u32 = 4096;

/// 默认超时时间（60秒）
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);

/// 连接池空闲超时（30秒）
pub const POOL_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

/// 连接池最大空闲连接数
pub const POOL_MAX_IDLE_PER_HOST: usize = 10;

/// ==================== 系统提示词 ====================

/// 默认系统提示词
pub const DEFAULT_SYSTEM_PROMPT: &str = "你是一个智能助手，请用简洁清晰的语言回答用户的问题。";

/// ==================== 对话历史 ====================

/// 最大历史消息数
pub const MAX_HISTORY_MESSAGES: usize = 20;

/// ==================== 工具调用 ====================

/// 默认工具调用超时时间（30秒）
pub const TOOL_CALL_TIMEOUT: Duration = Duration::from_secs(30);

/// ==================== 重试配置 ====================

/// 最大重试次数
pub const MAX_RETRY_COUNT: u32 = 3;

/// 初始重试间隔（1秒）
pub const INITIAL_RETRY_DELAY: Duration = Duration::from_secs(1);

/// 最大重试间隔（30秒）
pub const MAX_RETRY_DELAY: Duration = Duration::from_secs(30);

/// ==================== 速率限制 ====================

/// 默认请求速率限制（每分钟）
pub const DEFAULT_RATE_LIMIT_PER_MINUTE: u32 = 60;

/// 速率限制突发值
pub const RATE_LIMIT_BURST: u32 = 10;

/// ==================== API 版本 ====================

/// Anthropic API 版本
pub const ANTHROPIC_API_VERSION: &str = "2023-06-01";

/// ==================== 辅助函数 ====================

/// 获取 Provider 的默认模型
///
/// # 参数说明
/// * `provider_name` - Provider 名称
///
/// # 返回值
/// 默认模型名称
pub fn get_default_model(provider_name: &str) -> &'static str {
    match provider_name {
        PROVIDER_ANTHROPIC => ANTHROPIC_DEFAULT_MODEL,
        PROVIDER_OPENAI => OPENAI_DEFAULT_MODEL,
        PROVIDER_DEEPSEEK => DEEPSEEK_DEFAULT_MODEL,
        PROVIDER_MINIMAX => MINIMAX_DEFAULT_MODEL,
        _ => OPENAI_DEFAULT_MODEL,
    }
}

/// 获取 Provider 的 API 基础 URL
///
/// # 参数说明
/// * `provider_name` - Provider 名称
///
/// # 返回值
/// API 基础 URL
pub fn get_base_url(provider_name: &str) -> &'static str {
    match provider_name {
        PROVIDER_OPENAI => OPENAI_BASE_URL,
        PROVIDER_ANTHROPIC => ANTHROPIC_BASE_URL,
        PROVIDER_DEEPSEEK => DEEPSEEK_BASE_URL,
        PROVIDER_MINIMAX => MINIMAX_BASE_URL,
        _ => OPENAI_BASE_URL,
    }
}

/// 检查 Provider 名称是否有效
///
/// # 参数说明
/// * `name` - Provider 名称
///
/// # 返回值
/// 如果有效返回 true
pub fn is_valid_provider(name: &str) -> bool {
    matches!(
        name,
        PROVIDER_ANTHROPIC | PROVIDER_OPENAI | PROVIDER_DEEPSEEK | PROVIDER_MINIMAX
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_default_model() {
        assert_eq!(get_default_model(PROVIDER_OPENAI), OPENAI_DEFAULT_MODEL);
        assert_eq!(get_default_model(PROVIDER_ANTHROPIC), ANTHROPIC_DEFAULT_MODEL);
        assert_eq!(get_default_model(PROVIDER_DEEPSEEK), DEEPSEEK_DEFAULT_MODEL);
        assert_eq!(get_default_model(PROVIDER_MINIMAX), MINIMAX_DEFAULT_MODEL);
        assert_eq!(get_default_model("unknown"), OPENAI_DEFAULT_MODEL);
    }

    #[test]
    fn test_get_base_url() {
        assert_eq!(get_base_url(PROVIDER_OPENAI), OPENAI_BASE_URL);
        assert_eq!(get_base_url(PROVIDER_ANTHROPIC), ANTHROPIC_BASE_URL);
        assert_eq!(get_base_url(PROVIDER_DEEPSEEK), DEEPSEEK_BASE_URL);
        assert_eq!(get_base_url(PROVIDER_MINIMAX), MINIMAX_BASE_URL);
        assert_eq!(get_base_url("unknown"), OPENAI_BASE_URL);
    }

    #[test]
    fn test_is_valid_provider() {
        assert!(is_valid_provider(PROVIDER_OPENAI));
        assert!(is_valid_provider(PROVIDER_ANTHROPIC));
        assert!(is_valid_provider(PROVIDER_DEEPSEEK));
        assert!(is_valid_provider(PROVIDER_MINIMAX));
        assert!(!is_valid_provider("invalid"));
        assert!(!is_valid_provider(""));
    }
}
