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
