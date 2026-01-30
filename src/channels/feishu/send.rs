//! 飞书消息发送模块
//!
//! 本模块实现了向飞书发送消息的功能。

use serde::{Deserialize, Serialize};
use tracing::{debug, info};
use async_trait::async_trait;

use super::client::{FeishuClient, FeishuCredentials};
use crate::channels::traits::MessageSender;
use crate::core::message::types::OutboundMessage;
use crate::infra::error::Result;

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
