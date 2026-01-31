//! 配置管理系统模块
//!
//! 本模块负责加载和管理系统配置。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::{env, fs};

/// 主配置结构
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    /// AI 配置
    #[serde(default)]
    pub ai: AiConfig,
    /// 渠道配置
    #[serde(default)]
    pub channels: std::collections::HashMap<String, ChannelConfig>,
    /// 日志配置
    #[serde(default)]
    pub logging: LoggingConfigConfig,
    /// 安全审计配置
    #[serde(default)]
    pub security: SecurityConfig,
    /// 会话配置
    #[serde(default)]
    pub session: SessionConfig,
}

/// 会话配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionConfig {
    /// 会话过期时间（秒）
    pub expire_seconds: Option<u64>,
    /// 最大历史消息数
    pub max_history: Option<usize>,
    /// 是否持久化到数据库
    pub persist_enabled: Option<bool>,
    /// 数据库路径
    pub db_path: Option<String>,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            expire_seconds: Some(3600),
            max_history: Some(50),
            persist_enabled: Some(true),
            db_path: Some("data/clawdbot.db".to_string()),
        }
    }
}

/// AI 配置
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AiConfig {
    /// 默认 Provider
    pub default_provider: Option<String>,
    /// Provider 配置
    pub providers: std::collections::HashMap<String, ProviderConfig>,
}

/// AI Provider 配置
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProviderConfig {
    /// API Key
    pub api_key: Option<String>,
    /// Group ID（MiniMax 等需要）
    pub group_id: Option<String>,
    /// Base URL
    pub base_url: Option<String>,
    /// 模型名称
    pub model: Option<String>,
    /// 温度参数
    pub temperature: Option<f32>,
    /// 最大 Token 数
    pub max_tokens: Option<u32>,
}

/// 渠道配置
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChannelConfig {
    /// 渠道类型
    pub channel_type: String,
    /// 启用状态
    pub enabled: bool,
    /// 凭证配置
    pub credentials: std::collections::HashMap<String, String>,
}

/// 日志配置
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LoggingConfigConfig {
    /// 日志级别
    pub level: Option<String>,
    /// 日志文件路径
    pub file_path: Option<PathBuf>,
}

/// 安全审计配置
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SecurityConfig {
    /// 是否启用审计
    #[serde(default)]
    pub enabled: bool,
    /// 是否存储原始消息内容
    #[serde(default)]
    pub audit_store_original: bool,
    /// 审计日志保留天数
    #[serde(default)]
    pub audit_retention_days: Option<u32>,
    /// 是否启用邮件告警
    #[serde(default)]
    pub alert_enabled: bool,
    /// 邮件告警阈值（每秒事件数）
    #[serde(default)]
    pub alert_threshold: Option<u32>,
    /// SMTP 服务器地址
    pub smtp_server: Option<String>,
    /// SMTP 端口
    pub smtp_port: Option<u16>,
    /// SMTP 用户名
    pub smtp_username: Option<String>,
    /// SMTP 密码
    pub smtp_password: Option<String>,
    /// 发件人邮箱
    pub from_address: Option<String>,
    /// 收件人邮箱
    pub to_addresses: Option<Vec<String>>,
    /// 是否使用 TLS
    #[serde(default)]
    pub smtp_tls: bool,
}

/// 配置加载器
#[derive(Debug, Clone)]
pub struct ConfigLoader;

impl ConfigLoader {
    /// 创建新的配置加载器
    pub fn new() -> Self {
        Self
    }

    /// 加载配置
    pub async fn load(&self, path: &str) -> Result<Config, super::error::Error> {
        tracing::info!(path = path, "加载配置文件");

        // 检查文件是否存在
        if !PathBuf::from(path).exists() {
            tracing::warn!(path = path, "配置文件不存在，使用默认配置");
            return Ok(Config::default());
        }

        // 读取文件内容
        let content = fs::read_to_string(path)
            .map_err(|e| super::error::Error::Config(format!("读取配置文件失败: {}", e)))?;

        // 解析 TOML
        let mut config: Config = toml::from_str(&content)
            .map_err(|e| super::error::Error::Config(format!("解析配置文件失败: {}", e)))?;

        // 环境变量替换
        self.substitute_env_vars(&mut config);

        tracing::info!("配置加载成功");
        Ok(config)
    }

    /// 替换环境变量
    ///
    /// 将 `${VAR_NAME}` 格式的字符串替换为对应的环境变量值
    fn substitute_env_vars(&self, config: &mut Config) {
        // 替换 AI Provider 配置中的环境变量
        for (_, provider) in &mut config.ai.providers {
            if let Some(api_key) = &provider.api_key {
                provider.api_key = Some(self.replace_env_vars(api_key));
            }
            if let Some(group_id) = &provider.group_id {
                provider.group_id = Some(self.replace_env_vars(group_id));
            }
            if let Some(base_url) = &provider.base_url {
                provider.base_url = Some(self.replace_env_vars(base_url));
            }
            if let Some(model) = &provider.model {
                provider.model = Some(self.replace_env_vars(model));
            }
            if let Some(temperature) = &provider.temperature {
                // 温度可能是 `${TEMPERATURE}` 格式
                let temp_str = temperature.to_string();
                if temp_str.starts_with("${") {
                    provider.temperature = self.replace_env_vars(&temp_str).parse().ok().or(Some(*temperature));
                }
            }
            if let Some(max_tokens) = &provider.max_tokens {
                // Max tokens 可能是 `${MAX_TOKENS}` 格式
                let tokens_str = max_tokens.to_string();
                if tokens_str.starts_with("${") {
                    provider.max_tokens = self.replace_env_vars(&tokens_str).parse().ok().or(Some(*max_tokens));
                }
            }
        }

        // 替换渠道配置中的环境变量
        for (_, channel) in &mut config.channels {
            for (_, value) in &mut channel.credentials {
                *value = self.replace_env_vars(value);
            }
        }
    }

    /// 替换字符串中的环境变量
    fn replace_env_vars(&self, input: &str) -> String {
        let pattern = r"\$\{([^}]+)\}";

        // 使用正则表达式替换环境变量
        let re = regex::Regex::new(pattern).unwrap();
        let result = re.replace_all(input, |caps: &regex::Captures| {
            let var_name = &caps[1];
            env::var(var_name).unwrap_or_else(|_| caps[0].to_string())
        });

        result.to_string()
    }
}
