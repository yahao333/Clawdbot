use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use async_trait::async_trait;
use crate::infra::error::Result;
use super::types::OutboundMessage;

/// 消息发送器 Trait
///
/// 定义发送消息的统一接口
#[async_trait]
pub trait MessageSender: Send + Sync {
    /// 发送消息
    ///
    /// # 参数说明
    /// * `message` - 出站消息
    ///
    /// # 返回值
    /// 发送成功返回消息 ID
    async fn send(&self, message: OutboundMessage) -> Result<String>;
}

/// 统一消息发送器
///
/// 负责根据渠道类型将消息路由到对应的发送器
#[derive(Clone, Default)]
pub struct UnifiedMessageSender {
    senders: Arc<RwLock<HashMap<String, Arc<dyn MessageSender>>>>,
}

impl UnifiedMessageSender {
    /// 创建新的统一发送器
    pub fn new() -> Self {
        Self {
            senders: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// 注册发送器
    pub async fn register(&self, channel: &str, sender: Arc<dyn MessageSender>) {
        let mut senders = self.senders.write().await;
        senders.insert(channel.to_string(), sender);
    }
}

#[async_trait]
impl MessageSender for UnifiedMessageSender {
    async fn send(&self, message: OutboundMessage) -> Result<String> {
        let senders = self.senders.read().await;
        if let Some(sender) = senders.get(&message.channel) {
            sender.send(message).await
        } else {
            Err(crate::infra::error::Error::Channel(format!(
                "未找到渠道 '{}' 的发送器",
                message.channel
            )))
        }
    }
}
