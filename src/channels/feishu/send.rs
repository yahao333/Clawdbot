//! 飞书消息发送模块
//!
//! 本模块实现了向飞书发送消息的功能。
//!
//! # 支持的消息类型
//! - 文本消息 (text)
//! - 富文本消息 (post)
//! - 卡片消息 (interactive)
//! - Markdown 卡片 (card)
//! - 图片消息 (image)
//!
//! # 卡片消息
//! 飞书支持两种卡片消息格式：
//! 1. Interactive Card - 包含按钮、表单等交互元素
//! 2. Markdown Card - 支持 Markdown 语法的简单卡片

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::{debug, info};
use async_trait::async_trait;

use super::client::{FeishuClient, FeishuCredentials};
use crate::channels::traits::MessageSender;
use crate::core::message::types::OutboundMessage;
use crate::infra::error::Result;

/// 卡片按钮配置
///
/// 用于快速构建卡片消息中的按钮
#[derive(Debug, Clone)]
pub struct CardButton {
    /// 按钮显示文本
    pub text: String,
    /// 动作 ID（用于回调识别）
    pub action_id: String,
    /// 按钮样式 (primary, default, danger)
    pub style: Option<String>,
}

impl CardButton {
    /// 创建新按钮
    pub fn new(text: &str, action_id: &str) -> Self {
        Self {
            text: text.to_string(),
            action_id: action_id.to_string(),
            style: None,
        }
    }

    /// 设置为主要按钮（蓝色）
    pub fn primary(mut self) -> Self {
        self.style = Some("primary".to_string());
        self
    }

    /// 设置为危险按钮（红色）
    pub fn danger(mut self) -> Self {
        self.style = Some("danger".to_string());
        self
    }

    /// 设置为默认样式
    pub fn default(mut self) -> Self {
        self.style = Some("default".to_string());
        self
    }
}

/// 消息接收者
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MessageReceiver {
    /// 用户 ID（Open ID）
    OpenId(String),
    /// 用户 ID（Union ID）
    UnionId(String),
    /// 用户 ID（User ID）
    UserId(String),
    /// 群聊 ID
    ChatId(String),
    /// 部门 ID
    DepartmentId(String),
    /// 邮箱
    Email(String),
}

impl MessageReceiver {
    pub fn id_type(&self) -> &'static str {
        match self {
            MessageReceiver::OpenId(_) => "open_id",
            MessageReceiver::UnionId(_) => "union_id",
            MessageReceiver::UserId(_) => "user_id",
            MessageReceiver::ChatId(_) => "chat_id",
            MessageReceiver::DepartmentId(_) => "department_id",
            MessageReceiver::Email(_) => "email",
        }
    }

    pub fn id_value(&self) -> &str {
        match self {
            MessageReceiver::OpenId(id) => id,
            MessageReceiver::UnionId(id) => id,
            MessageReceiver::UserId(id) => id,
            MessageReceiver::ChatId(id) => id,
            MessageReceiver::DepartmentId(id) => id,
            MessageReceiver::Email(id) => id,
        }
    }
}

/// 发送消息响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendMessageResponse {
    /// 消息 ID
    pub message_id: String,
    /// 根消息 ID
    pub root_id: Option<String>,
    /// 父消息 ID
    pub parent_id: Option<String>,
}

/// 飞书消息发送器
#[derive(Clone, Debug)]
pub struct FeishuMessageSender {
    client: FeishuClient,
}

impl FeishuMessageSender {
    pub fn new(credentials: FeishuCredentials) -> Self {
        Self {
            client: FeishuClient::new(credentials),
        }
    }

    pub fn from_client(client: FeishuClient) -> Self {
        Self { client }
    }

    /// 发送文本消息
    pub async fn send_text(&self, receiver: &MessageReceiver, text: &str) -> Result<SendMessageResponse> {
        let content = serde_json::json!({
            "text": text
        });
        self.send_raw(receiver, "text", content).await
    }

    /// 发送富文本消息
    pub async fn send_rich_text(&self, receiver: &MessageReceiver, content: &serde_json::Value) -> Result<SendMessageResponse> {
        self.send_raw(receiver, "post", content.clone()).await
    }

    /// 发送交互式卡片消息
    ///
    /// 飞书交互式卡片支持多种元素：
    /// - 按钮 (button)
    /// - 选择器 (select)
    /// - 文本输入 (input)
    /// - 分割线 (divider)
    /// - 等等
    ///
    /// # 参数说明
    /// * `receiver` - 消息接收者
    /// * `card` - 卡片内容 JSON
    ///
    /// # 示例
    /// ```json
    /// {
    ///   "config": {
    ///     "wide_screen_mode": true
    ///   },
    ///   "elements": [
    ///     {"tag": "text", "text": "标题"},
    ///     {"tag": "button", "text": "点击我", "action_id": "btn_click"}
    ///   ]
    /// }
    /// ```
    pub async fn send_card(&self, receiver: &MessageReceiver, card: &Value) -> Result<SendMessageResponse> {
        self.send_raw(receiver, "interactive", card.clone()).await
    }

    /// 发送 Markdown 卡片消息
    ///
    /// Markdown 卡片是一种简单卡片，支持 Markdown 语法
    ///
    /// # 参数说明
    /// * `receiver` - 消息接收者
    /// * `markdown` - Markdown 内容
    pub async fn send_markdown_card(&self, receiver: &MessageReceiver, markdown: &str) -> Result<SendMessageResponse> {
        let card = serde_json::json!({
            "config": {
                "wide_screen_mode": true
            },
            "elements": [
                {
                    "tag": "markdown",
                    "content": markdown
                }
            ]
        });
        self.send_card(receiver, &card).await
    }

    /// 发送带按钮的卡片消息
    ///
    /// 快速创建包含标题、文本和多个按钮的卡片
    pub async fn send_card_with_buttons(
        &self,
        receiver: &MessageReceiver,
        title: &str,
        text: &str,
        buttons: &[CardButton],
    ) -> Result<SendMessageResponse> {
        let elements: Vec<Value> = buttons
            .iter()
            .map(|btn| {
                serde_json::json!({
                    "tag": "button",
                    "text": {
                        "tag": "plain_text",
                        "content": btn.text
                    },
                    "action_id": btn.action_id,
                    "style": btn.style.as_deref().unwrap_or("default")
                })
            })
            .collect();

        // 构建完整的 elements 数组
        let mut all_elements = Vec::new();

        // 添加标题
        all_elements.push(serde_json::json!({
            "tag": "text",
            "text": {
                "tag": "plain_text",
                "content": title
            }
        }));

        // 添加分割线
        all_elements.push(serde_json::json!({
            "tag": "divider"
        }));

        // 添加文本
        all_elements.push(serde_json::json!({
            "tag": "markdown",
            "content": text
        }));

        // 添加分割线
        all_elements.push(serde_json::json!({
            "tag": "divider"
        }));

        // 添加按钮
        all_elements.extend(elements);

        let card = serde_json::json!({
            "config": {
                "wide_screen_mode": true
            },
            "elements": all_elements
        });

        self.send_card(receiver, &card).await
    }

    /// 发送图片消息
    ///
    /// 需要先上传图片获取 image_key
    pub async fn send_image(&self, receiver: &MessageReceiver, image_key: &str) -> Result<SendMessageResponse> {
        let content = serde_json::json!({
            "image_key": image_key
        });
        self.send_raw(receiver, "image", content).await
    }

    /// 发送回复消息
    ///
    /// 在指定消息下创建回复
    pub async fn send_reply(
        &self,
        receiver: &MessageReceiver,
        parent_id: &str,
        msg_type: &str,
        content: &Value,
    ) -> Result<SendMessageResponse> {
        let path = format!(
            "/im/v1/messages/{}/reply?receive_id_type={}",
            parent_id,
            receiver.id_type()
        );

        #[derive(Serialize)]
        struct RequestBody<'a> {
            receive_id: &'a str,
            msg_type: &'a str,
            content: String,
        }

        let body = RequestBody {
            receive_id: receiver.id_value(),
            msg_type,
            content: content.to_string(),
        };

        let response: SendMessageResponse = self.client.request("POST", &path, Some(body)).await?;
        info!(message_id = %response.message_id, "回复消息发送成功");
        Ok(response)
    }

    /// 发送原始消息
    async fn send_raw(&self, receiver: &MessageReceiver, msg_type: &str, content: serde_json::Value) -> Result<SendMessageResponse> {
        let path = format!("/im/v1/messages?receive_id_type={}", receiver.id_type());
        
        #[derive(Serialize)]
        struct RequestBody<'a> {
            receive_id: &'a str,
            msg_type: &'a str,
            content: String,
        }

        let body = RequestBody {
            receive_id: receiver.id_value(),
            msg_type,
            content: content.to_string(),
        };

        let response: SendMessageResponse = self.client.request("POST", &path, Some(body)).await?;
        
        info!(message_id = %response.message_id, "消息发送成功");
        Ok(response)
    }
}

#[async_trait]
impl MessageSender for FeishuMessageSender {
    async fn send(&self, message: OutboundMessage) -> Result<String> {
        let receiver = if message.target_id.starts_with("oc_") {
            MessageReceiver::ChatId(message.target_id)
        } else {
            MessageReceiver::OpenId(message.target_id)
        };

        let response = if let Some(text) = message.content.text {
            self.send_text(&receiver, &text).await?
        } else if let Some(rich_text) = message.content.rich_text {
            let content = serde_json::from_str(&rich_text.content)
                .map_err(|e| crate::infra::error::Error::Serialization(e.to_string()))?;
            self.send_rich_text(&receiver, &content).await?
        } else {
            return Err(crate::infra::error::Error::Channel("不支持的消息内容类型".to_string()));
        };
        
        Ok(response.message_id)
    }
}
