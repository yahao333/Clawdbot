//! 飞书事件处理模块
//!
//! 处理飞书推送的事件，包括：
//! 1. 验证请求签名（安全性）
//! 2. 解析事件数据
//! 3. 转换为内部消息格式
//!
//! # 事件类型
//! - `im.message.receive_v1`: 收到消息事件
//! - `im.message.message_read_v1`: 消息已读事件
//!
//! # 签名验证流程
//! ```
//! 1. 从请求头获取 timestamp 和 sign
//! 2. 拼接 timestamp + body
//! 3. 使用 HMAC-SHA256 计算签名
//! 4. 与请求头中的 sign 对比
//! ```

use chrono::Utc;
use serde::Deserialize;
use tracing::{debug, warn};

use crate::core::message::types::{InboundMessage, MessageContent, MessageSource, SenderInfo, TargetInfo};
use crate::infra::error::{Error, Result};

/// 飞书事件请求结构
///
/// 飞书 Webhook 推送的事件数据
#[derive(Debug, Deserialize)]
pub struct FeishuEventRequest {
    /// 事件类型
    #[serde(rename = "type")]
    pub event_type: String,
    /// 事件唯一标识
    pub event_id: String,
    /// 创建时间戳
    pub created_at: i64,
    /// 事件数据
    pub event: serde_json::Value,
}

/// 飞书消息事件
///
/// 收到消息事件的数据
#[derive(Debug, Deserialize)]
pub struct FeishuMessageEvent {
    /// 发送者
    pub sender: FeishuSender,
    /// 消息内容
    pub message: FeishuMessageBody,
}

/// 飞书消息体详情
#[derive(Debug, Deserialize)]
pub struct FeishuMessageBody {
    /// 消息 ID
    pub message_id: String,
    /// 根消息 ID（用于线程）
    pub root_id: Option<String>,
    /// 父消息 ID（用于回复）
    pub parent_id: Option<String>,
    /// 创建时间
    pub create_time: String,
    /// 会话 ID
    pub chat_id: String,
    /// 会话类型
    pub chat_type: String,
    /// 消息类型
    pub message_type: String,
    /// 消息内容（JSON 格式）
    pub content: String,
}

/// 飞书文本消息内容
#[derive(Debug, Deserialize)]
pub struct FeishuTextContent {
    /// 文本内容
    pub text: String,
}

/// 飞书图片消息内容
#[derive(Debug, Deserialize)]
pub struct FeishuImageContent {
    /// 图片密钥
    pub image_key: String,
    /// 图片宽度
    pub width: Option<u32>,
    /// 图片高度
    pub height: Option<u32>,
}

/// 消息接收处理结果
pub struct MessageReceiveResult {
    /// 转换后的消息
    pub message: InboundMessage,
    /// 是否需要发送已读回执
    pub need_read_receipt: bool,
}

/// 事件处理器 Trait
///
/// 定义事件处理的接口
#[async_trait::async_trait]
pub trait EventHandler: Send + Sync {
    /// 处理事件
    async fn handle(&self, event: &FeishuEventRequest) -> Result<()>;
}

/// 消息事件处理器
///
/// 处理消息相关的事件
#[derive(Clone, Debug)]
pub struct MessageEventHandler;

impl MessageEventHandler {
    /// 创建新的消息事件处理器
    pub fn new() -> Self {
        Self
    }

    /// 处理消息事件
    ///
    /// # 参数说明
    /// * `event` - 飞书事件请求
    ///
    /// # 返回值
    /// 处理结果（包含转换后的消息）
    pub async fn handle(&self, event: &FeishuEventRequest) -> Result<MessageReceiveResult> {
        debug!(event_type = %event.event_type, event_id = %event.event_id, "处理消息事件");

        // 检查事件类型
        if event.event_type != "im.message.receive_v1" {
            warn!(event_type = %event.event_type, "跳过非消息事件");
            return Err(Error::Channel(format!("不支持的事件类型: {}", event.event_type)));
        }

        // 解析消息事件数据
        let message_event: FeishuMessageEvent = serde_json::from_value(event.event.clone())
            .map_err(|e| Error::Serialization(e.to_string()))?;

        debug!(message_id = %message_event.message.message_id, "解析消息事件成功");

        // 转换为内部消息格式
        let message = self.convert_to_inbound(&message_event, &event.event_id)?;

        // 检查是否需要已读回执
        let need_read_receipt = message_event.sender.sender_type == "user";

        Ok(MessageReceiveResult {
            message,
            need_read_receipt,
        })
    }

    /// 转换为内部消息格式
    ///
    /// 将飞书消息转换为系统内部的 InboundMessage 格式
    ///
    /// # 参数说明
    /// * `event` - 飞书消息事件
    /// * `event_id` - 事件 ID
    ///
    /// # 返回值
    /// 转换后的消息
    fn convert_to_inbound(&self, event: &FeishuMessageEvent, event_id: &str) -> Result<InboundMessage> {
        // 解析消息内容
        let (content, msg_type) = self.parse_message_content(&event.message)?;

        // 确定发送者 ID
        let sender_id = event.sender.sender_id.open_id
            .as_ref()
            .or(event.sender.sender_id.user_id.as_ref())
            .or(event.sender.sender_id.union_id.as_ref())
            .map(|s| s.clone())
            .unwrap_or_else(|| "unknown".to_string());

        // 确定消息来源类型
        let source = match event.message.chat_type.as_str() {
            "p2p" => MessageSource::Direct,
            "group" => MessageSource::Group,
            _ => MessageSource::Direct,
        };

        // 创建目标信息
        // 使用 chat_id 作为目标 ID，这对于发送消息很重要
        let target_id = event.message.chat_id.clone();

        // 构建消息
        Ok(InboundMessage {
            id: event.message.message_id.clone(),
            channel: "feishu".to_string(),
            source,
            sender: SenderInfo {
                id: sender_id,
                username: None, // 需要额外 API 调用获取
                display_name: None,
                avatar_url: None,
            },
            target: TargetInfo {
                id: target_id,
                target_type: Some(event.message.chat_type.clone()),
                name: None,
            },
            content,
            raw: serde_json::json!({
                "event_id": event_id,
                "msg_type": msg_type,
                "root_id": event.message.root_id,
                "parent_id": event.message.parent_id,
            }),
            timestamp: Utc::now(),
            metadata: crate::core::message::types::MessageMetadata {
                is_edited: false,
                is_reply: event.message.parent_id.is_some(),
                forward_count: 0,
                thread_id: event.message.root_id.clone().or(event.message.parent_id.clone()),
            },
        })
    }

    /// 解析消息内容
    ///
    /// 根据消息类型解析内容
    fn parse_message_content(&self, message: &FeishuMessageBody) -> Result<(MessageContent, String)> {
        let msg_type = message.message_type.clone();

        match msg_type.as_str() {
            "text" => {
                // 文本消息
                let text_content: FeishuTextContent = serde_json::from_str(&message.content)
                    .map_err(|e| Error::Serialization(e.to_string()))?;

                Ok((MessageContent::text(&text_content.text), msg_type))
            }
            "image" => {
                // 图片消息
                let image_content: FeishuImageContent = serde_json::from_str(&message.content)
                    .map_err(|e| Error::Serialization(e.to_string()))?;

                Ok((MessageContent {
                    text: None,
                    rich_text: None,
                    attachments: vec![crate::core::message::types::Attachment {
                        kind: crate::core::message::types::AttachmentKind::Image(
                            crate::core::message::types::ImageAttachment {
                                width: image_content.width,
                                height: image_content.height,
                                format: "unknown".to_string(),
                                is_animated: false,
                            },
                        ),
                        filename: None,
                        mime_type: "image".to_string(),
                        size: 0,
                        url: Some(image_content.image_key),
                        local_path: None,
                    }],
                    quoted_message: None,
                }, msg_type))
            }
            _ => {
                // 其他类型消息
                warn!(msg_type = %msg_type, "收到未知消息类型");
                Ok((MessageContent::text(&format!("[未知消息类型: {}]", msg_type)), msg_type))
            }
        }
    }
}

/// 签名验证工具
///
/// 用于验证飞书事件的签名
pub struct SignatureVerifier;

impl SignatureVerifier {
    /// 验证签名
    ///
    /// # 参数说明
    /// * `timestamp` - 时间戳
    /// * `sign` - 签名
    /// * `body` - 请求体
    /// * `secret` - 加密密钥
    ///
    /// # 返回值
    /// 验证成功返回 `Ok(())`
    pub fn verify(timestamp: &str, sign: &str, body: &str, secret: &str) -> Result<()> {
        // TODO: 实现签名验证
        // 1. 拼接 timestamp + body
        // 2. 使用 HMAC-SHA256 计算签名
        // 3. 与请求头中的 sign 对比

        tracing::warn!("飞书签名验证尚未实现，跳过验证");

        Ok(())
    }
}

/// 空事件处理器
///
/// 用于不需要处理的占位
#[derive(Clone, Debug, Default)]
pub struct NoopEventHandler;

#[async_trait::async_trait]
impl EventHandler for NoopEventHandler {
    async fn handle(&self, _event: &FeishuEventRequest) -> Result<()> {
        Ok(())
    }
}

/// 辅助 trait，为 handlers 添加方法
trait FeishuClientExt {}

/// 飞书消息列表响应
///
/// 用于长链接轮询获取消息列表
#[derive(Debug, Deserialize)]
pub struct MessageListResponse {
    /// 响应码
    pub code: i64,
    /// 响应消息
    pub msg: String,
    /// 响应数据
    pub data: Option<MessageListData>,
}

/// 消息列表数据
#[derive(Debug, Deserialize)]
pub struct MessageListData {
    /// 消息列表
    #[serde(default)]
    pub items: Vec<FeishuMessageItem>,
    /// 分页令牌
    pub page_token: Option<String>,
    /// 是否有更多数据
    pub has_more: bool,
}

/// 飞书消息项
///
/// 单条消息数据
#[derive(Debug, Deserialize, Clone)]
pub struct FeishuMessageItem {
    /// 消息 ID
    pub message_id: String,
    /// 根消息 ID（用于线程）
    #[serde(default)]
    pub root_id: Option<String>,
    /// 父消息 ID（用于回复）
    #[serde(default)]
    pub parent_id: Option<String>,
    /// 消息类型
    #[serde(rename = "type")]
    pub msg_type: String,
    /// 创建时间
    #[serde(default)]
    pub create_time: String,
    /// 更新时间
    #[serde(default)]
    pub update_time: String,
    /// 创建时间（毫秒）
    #[serde(default)]
    pub create_time_ms: i64,
    /// 会话 ID
    #[serde(default)]
    pub chat_id: Option<String>,
    /// 发送者
    #[serde(default)]
    pub sender: Option<FeishuSender>,
    /// 消息体
    #[serde(default)]
    pub body: Option<FeishuBody>,
}

/// 飞书发送者
#[derive(Debug, Deserialize, Clone)]
pub struct FeishuSender {
    /// 发送者 ID
    pub sender_id: FeishuSenderId,
    /// 发送者类型
    pub sender_type: String,
}

/// 飞书发送者 ID
#[derive(Debug, Deserialize, Clone)]
pub struct FeishuSenderId {
    /// Open ID
    #[serde(default)]
    pub open_id: Option<String>,
    /// User ID
    #[serde(default)]
    pub user_id: Option<String>,
    /// Union ID
    #[serde(default)]
    pub union_id: Option<String>,
}

/// 飞书消息体
#[derive(Debug, Deserialize, Clone)]
pub struct FeishuBody {
    /// 消息内容
    #[serde(default)]
    pub content: String,
}
