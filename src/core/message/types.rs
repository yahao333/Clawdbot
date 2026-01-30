//! 消息类型定义模块
//!
//! 定义所有与消息相关的类型结构体，包括：
//! - 入站消息（从渠道接收的消息）
//! - 出站消息（发送给渠道的消息）
//! - 消息内容（文本、附件等）
//! - 发送者/目标信息
//!
//! # 使用示例
//! ```rust
//! let inbound = InboundMessage {
//!     id: "msg_123".to_string(),
//!     content: MessageContent::text("你好"),
//!     sender: SenderInfo::new("user_456"),
//!     target: TargetInfo::new("channel_789"),
//!     timestamp: Utc::now(),
//! };
//! ```

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 消息来源类型
///
/// 表示消息来自哪种类型的聊天
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageSource {
    /// 私聊（一对一对话）
    Direct,
    /// 群组聊天
    Group,
    /// 频道（广播频道）
    Channel,
}

/// 消息内容
///
/// 支持纯文本、富文本、附件等多种内容格式
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MessageContent {
    /// 纯文本内容
    pub text: Option<String>,
    /// 富文本内容（Markdown 等）
    pub rich_text: Option<RichText>,
    /// 附件列表
    pub attachments: Vec<Attachment>,
    /// 引用消息
    pub quoted_message: Option<QuotedMessage>,
}

/// 富文本内容
///
/// 支持多种格式的富文本
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RichText {
    /// 格式类型（如 "markdown", "html"）
    pub format: String,
    /// 富文本内容
    pub content: String,
}

/// 附件
///
/// 消息附带的各种文件（图片、音频、视频、文档等）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    /// 附件类型
    pub kind: AttachmentKind,
    /// 文件名
    pub filename: Option<String>,
    /// MIME 类型
    pub mime_type: String,
    /// 文件大小（字节）
    pub size: u64,
    /// 下载 URL
    pub url: Option<String>,
    /// 本地路径
    pub local_path: Option<std::path::PathBuf>,
}

/// 附件类型枚举
///
/// 不同类型的附件有不同的处理方式
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AttachmentKind {
    /// 图片附件
    Image(ImageAttachment),
    /// 视频附件
    Video(VideoAttachment),
    /// 音频附件
    Audio(AudioAttachment),
    /// 普通文件
    File(FileAttachment),
    /// 表情包/贴纸
    Sticker(StickerAttachment),
}

/// 图片附件
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageAttachment {
    /// 图片宽度（像素）
    pub width: Option<u32>,
    /// 图片高度（像素）
    pub height: Option<u32>,
    /// 图片格式（JPEG, PNG, GIF, WebP 等）
    pub format: String,
    /// 是否是动图
    pub is_animated: bool,
}

/// 视频附件
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoAttachment {
    /// 视频时长（秒）
    pub duration: Option<u64>,
    /// 视频宽度
    pub width: Option<u32>,
    /// 视频高度
    pub height: Option<u32>,
    /// 视频格式
    pub format: Option<String>,
}

/// 音频附件
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioAttachment {
    /// 音频时长（秒）
    pub duration: Option<u64>,
    /// 音频格式
    pub format: Option<String>,
}

/// 普通文件附件
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileAttachment {
    /// 文件扩展名
    pub extension: Option<String>,
}

/// 表情包附件
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StickerAttachment {
    /// 表情包 ID
    pub id: Option<String>,
    /// 表情包名称
    pub name: Option<String>,
}

/// 引用消息
///
/// 回复或引用其他消息时使用
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotedMessage {
    /// 被引用消息的 ID
    pub message_id: String,
    /// 被引用消息的内容
    pub content: String,
    /// 被引用消息的发送者
    pub sender: SenderInfo,
}

/// 发送者信息
///
/// 标识消息的发送者
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SenderInfo {
    /// 发送者唯一 ID
    pub id: String,
    /// 发送者用户名（如有）
    pub username: Option<String>,
    /// 发送者显示名称
    pub display_name: Option<String>,
    /// 发送者头像 URL（可选）
    pub avatar_url: Option<String>,
}

/// 目标信息
///
/// 标识消息发送的目标（聊天、频道、群组等）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TargetInfo {
    /// 目标唯一 ID
    pub id: String,
    /// 目标类型（群组、频道等）
    pub target_type: Option<String>,
    /// 目标名称
    pub name: Option<String>,
}

/// 消息元数据
///
/// 消息的附加信息
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MessageMetadata {
    /// 是否为编辑消息
    pub is_edited: bool,
    /// 是否为回复
    pub is_reply: bool,
    /// 转发次数
    pub forward_count: u32,
    /// 线程 ID（用于线程回复）
    pub thread_id: Option<String>,
}

/// 入站消息结构体
///
/// 从渠道接收到的消息
///
/// # 字段说明
/// - `id`: 消息唯一 ID
/// - `channel`: 渠道类型（飞书、Telegram 等）
/// - `source`: 消息来源类型（私聊、群聊、频道）
/// - `sender`: 发送者信息
/// - `target`: 目标信息
/// - `content`: 消息内容
/// - `raw`: 原始消息数据（用于调试）
/// - `timestamp`: 消息时间戳
/// - `metadata`: 消息元数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboundMessage {
    /// 消息唯一 ID
    pub id: String,
    /// 渠道类型（如 "feishu", "telegram"）
    pub channel: String,
    /// 消息来源类型
    pub source: MessageSource,
    /// 发送者信息
    pub sender: SenderInfo,
    /// 目标信息
    pub target: TargetInfo,
    /// 消息内容
    pub content: MessageContent,
    /// 原始消息数据（用于调试）
    pub raw: serde_json::Value,
    /// 消息时间戳
    pub timestamp: DateTime<Utc>,
    /// 消息元数据
    pub metadata: MessageMetadata,
}

/// 出站消息结构体
///
/// 发送给渠道的消息
///
/// # 与 InboundMessage 的区别
/// - 只有必要字段，简化发送
/// - 支持更多发送选项
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OutboundMessage {
    /// 目标 ID（聊天 ID、频道 ID 等）
    pub target_id: String,
    /// 渠道类型（用于路由消息）
    pub channel: String,
    /// 消息内容
    pub content: MessageContent,
    /// 线程 ID（用于线程回复）
    pub thread_id: Option<String>,
    /// 是否启用消息推送（飞书特有）
    pub enable_push: Option<bool>,
    /// 消息卡片配置（富文本消息）
    pub card: Option<MessageCard>,
}

/// 消息卡片配置
///
/// 用于发送富文本消息卡片（飞书、钉钉等平台支持）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MessageCard {
    /// 卡片模板类型
    pub template_type: String,
    /// 卡片数据
    pub data: HashMap<String, serde_json::Value>,
}

impl InboundMessage {
    /// 创建一个简单的文本入站消息（用于测试）
    ///
    /// # 参数说明
    /// * `id` - 消息 ID
    /// * `text` - 文本内容
    /// * `sender_id` - 发送者 ID
    /// * `target_id` - 目标 ID
    /// * `channel` - 渠道类型
    ///
    /// # 返回值
    /// 创建的入站消息
    ///
    /// # 示例
    /// ```rust
    /// let msg = InboundMessage::simple(
    ///     "msg_001",
    ///     "你好",
    ///     "user_123",
    ///     "chat_456",
    ///     "feishu",
    /// );
    /// ```
    pub fn simple(
        id: &str,
        text: &str,
        sender_id: &str,
        target_id: &str,
        channel: &str,
    ) -> Self {
        Self {
            id: id.to_string(),
            channel: channel.to_string(),
            source: MessageSource::Direct,
            sender: SenderInfo {
                id: sender_id.to_string(),
                ..Default::default()
            },
            target: TargetInfo {
                id: target_id.to_string(),
                ..Default::default()
            },
            content: MessageContent {
                text: Some(text.to_string()),
                ..Default::default()
            },
            raw: serde_json::json!({}),
            timestamp: Utc::now(),
            metadata: MessageMetadata::default(),
        }
    }
}

impl MessageContent {
    /// 创建一个简单的文本消息内容
    ///
    /// # 参数说明
    /// * `text` - 文本内容
    ///
    /// # 返回值
    /// 创建的消息内容
    pub fn text(text: &str) -> Self {
        Self {
            text: Some(text.to_string()),
            ..Default::default()
        }
    }
}

impl SenderInfo {
    /// 创建发送者信息
    ///
    /// # 参数说明
    /// * `id` - 发送者 ID
    ///
    /// # 返回值
    /// 创建的发送者信息
    pub fn new(id: &str) -> Self {
        Self {
            id: id.to_string(),
            ..Default::default()
        }
    }
}

impl TargetInfo {
    /// 创建目标信息
    ///
    /// # 参数说明
    /// * `id` - 目标 ID
    ///
    /// # 返回值
    /// 创建的目标信息
    pub fn new(id: &str) -> Self {
        Self {
            id: id.to_string(),
            ..Default::default()
        }
    }
}
