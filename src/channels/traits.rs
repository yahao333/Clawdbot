//! 渠道 Trait 定义模块
//!
//! 定义渠道适配器的统一接口。
//!
//! # 设计原则
//! 1. 使用 `async-trait` 支持异步方法
//! 2. 所有方法返回 `Result` 类型
//! 3. 错误信息使用中文

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::core::message::types::{InboundMessage, OutboundMessage};
use crate::infra::error::Result;

/// 渠道类型枚举
///
/// 标识支持的即时通讯平台
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub enum ChannelType {
    /// 飞书
    Feishu,
    /// Telegram
    Telegram,
    /// Discord
    Discord,
    /// Slack
    Slack,
    /// Signal
    Signal,
    /// iMessage
    IMessage,
    /// Web（通用）
    Web,
    /// 自定义渠道
    Custom(String),
}

impl std::fmt::Display for ChannelType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChannelType::Feishu => write!(f, "feishu"),
            ChannelType::Telegram => write!(f, "telegram"),
            ChannelType::Discord => write!(f, "discord"),
            ChannelType::Slack => write!(f, "slack"),
            ChannelType::Signal => write!(f, "signal"),
            ChannelType::IMessage => write!(f, "imessage"),
            ChannelType::Web => write!(f, "web"),
            ChannelType::Custom(name) => write!(f, "{}", name),
        }
    }
}

/// 渠道能力
///
/// 描述渠道支持的功能
#[derive(Debug, Clone, Default)]
pub struct ChannelCapabilities {
    /// 是否支持发送消息
    pub send_message: bool,
    /// 是否支持发送媒体
    pub send_media: bool,
    /// 是否支持发送反应
    pub send_reaction: bool,
    /// 是否支持编辑消息
    pub edit_message: bool,
    /// 是否支持删除消息
    pub delete_message: bool,
    /// 是否支持线程回复
    pub thread_reply: bool,
    /// 是否支持轮询
    pub poll: bool,
    /// 最大媒体附件数
    pub max_media_attachments: usize,
    /// 最大消息长度
    pub max_message_length: usize,
}

/// 渠道适配器 Trait
///
/// 定义渠道适配器的统一接口
///
/// # 方法说明
/// - `channel_type()`: 返回渠道类型
/// - `capabilities()`: 返回渠道能力
/// - `connect()`: 连接到渠道
/// - `validate_credentials()`: 验证凭证
#[async_trait::async_trait]
pub trait ChannelAdapter: Send + Sync {
    /// 获取渠道类型
    fn channel_type(&self) -> ChannelType;

    /// 获取渠道能力
    fn capabilities(&self) -> &ChannelCapabilities;

    /// 连接到渠道
    ///
    /// # 参数说明
    /// * `config` - 渠道配置
    ///
    /// # 返回值
    /// 连接成功返回 `Ok(())`
    async fn connect(&self, config: &crate::infra::config::ChannelConfig) -> Result<()>;

    /// 验证凭证
    ///
    /// 验证渠道凭证是否有效
    ///
    /// # 返回值
    /// 验证通过返回 `true`
    async fn validate_credentials(&self) -> Result<bool>;
}

pub use crate::core::message::sender::MessageSender;

/// 已连接渠道 Trait
///
/// 定义已连接渠道的操作接口
///
/// # 方法说明
/// - `adapter()`: 获取渠道适配器引用
/// - `start_monitoring()`: 开始监控消息
/// - `send()`: 发送消息
/// - `send_media()`: 发送媒体
/// - `send_reaction()`: 发送反应
/// - `edit_message()`: 编辑消息
/// - `delete_message()`: 删除消息
/// - `disconnect()`: 断开连接
#[async_trait::async_trait]
pub trait ConnectedChannel: Send + Sync {
    /// 获取渠道适配器引用
    fn adapter(&self) -> &dyn ChannelAdapter;

    /// 开始监控消息
    ///
    /// # 返回值
    /// 消息流（异步迭代器）
    async fn start_monitoring(&self) -> Result<mpsc::Receiver<Result<InboundMessage>>>;

    /// 发送消息
    ///
    /// # 参数说明
    /// * `message` - 要发送的消息
    ///
    /// # 返回值
    /// 发送成功的消息 ID
    async fn send(&self, message: &OutboundMessage) -> Result<String>;

    /// 发送媒体
    ///
    /// # 参数说明
    /// * `message` - 要发送的媒体消息
    ///
    /// # 返回值
    /// 发送成功的消息 ID
    async fn send_media(&self, message: &OutboundMessage) -> Result<String>;

    /// 发送反应
    ///
    /// # 参数说明
    /// * `message_id` - 目标消息 ID
    /// * `emoji` - 反应表情
    async fn send_reaction(&self, message_id: &str, emoji: &str) -> Result<()>;

    /// 编辑消息
    ///
    /// # 参数说明
    /// * `message_id` - 要编辑的消息 ID
    /// * `content` - 新的内容
    async fn edit_message(&self, message_id: &str, content: &str) -> Result<()>;

    /// 删除消息
    ///
    /// # 参数说明
    /// * `message_id` - 要删除的消息 ID
    async fn delete_message(&self, message_id: &str) -> Result<()>;

    /// 断开连接
    async fn disconnect(&self);
}

/// 消息流类型
///
/// 接收消息的异步流
pub type MessageStream = mpsc::Receiver<Result<InboundMessage>>;
