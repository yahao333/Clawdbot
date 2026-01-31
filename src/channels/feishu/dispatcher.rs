//! 飞书事件分发器模块
//!
//! 提供灵活的 WebSocket 事件注册和处理机制。
//!
//! # 功能
//! 1. 事件处理器注册/注销
//! 2. 事件分发到对应处理器
//! 3. 支持异步处理
//!
//! # 使用示例
//! ```rust
//! let dispatcher = EventDispatcher::new();
//!
//! dispatcher.register("im.message.receive_v1", |data| {
//!     Box::pin(async move {
//!         // 处理消息事件
//!         Ok(())
//!     })
//! });
//! ```

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};
use serde_json::Value;

/// 事件处理器 Future 类型
///
/// 定义事件处理器的返回类型
type HandlerFuture = Pin<Box<dyn Future<Output = Result<(), String>> + Send>>;

/// 事件处理器创建函数类型
type EventHandlerFn = Box<dyn Fn(Value) -> HandlerFuture + Send + Sync>;

/// 事件分发器
///
/// 管理事件类型和对应处理器的注册表
#[derive(Clone, Default)]
pub struct EventDispatcher {
    /// 事件处理器注册表
    handlers: Arc<RwLock<HashMap<String, EventHandlerFn>>>,
}

impl EventDispatcher {
    /// 创建新的事件分发器
    pub fn new() -> Self {
        Self {
            handlers: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// 注册事件处理器
    ///
    /// # 参数说明
    /// * `event_type` - 事件类型，如 "im.message.receive_v1"
    /// * `handler` - 事件处理函数
    ///
    /// # 示例
    /// ```rust
    /// dispatcher.register("im.message.receive_v1", |data| {
    ///     Box::pin(async move {
    ///         // 处理消息事件
    ///         Ok(())
    ///     })
    /// });
    /// ```
    pub async fn register<F>(&self, event_type: &str, handler: F)
    where
        F: Fn(Value) -> HandlerFuture + Send + Sync + 'static,
    {
        let mut handlers = self.handlers.write().await;
        handlers.insert(event_type.to_string(), Box::new(handler));
        debug!(event_type = event_type, "事件处理器已注册");
    }

    /// 注销事件处理器
    ///
    /// # 参数说明
    /// * `event_type` - 事件类型
    pub async fn unregister(&self, event_type: &str) {
        let mut handlers = self.handlers.write().await;
        handlers.remove(event_type);
        debug!(event_type = event_type, "事件处理器已注销");
    }

    /// 检查是否有事件处理器注册
    pub async fn has_handler(&self, event_type: &str) -> bool {
        let handlers = self.handlers.read().await;
        handlers.contains_key(event_type)
    }

    /// 分发事件
    ///
    /// 根据事件类型查找处理器并调用
    ///
    /// # 参数说明
    /// * `event_type` - 事件类型
    /// * `event_data` - 事件数据
    ///
    /// # 返回值
    /// 成功返回 Ok(())，失败返回错误信息
    pub async fn dispatch(&self, event_type: &str, event_data: Value) -> Result<(), String> {
        let handlers = self.handlers.read().await;

        if let Some(handler) = handlers.get(event_type) {
            debug!(event_type = event_type, "找到事件处理器");
            let mut future: HandlerFuture = handler(event_data);
            match future.as_mut().await {
                Ok(()) => {
                    debug!(event_type = event_type, "事件处理成功");
                    Ok(())
                }
                Err(e) => {
                    error!(event_type = event_type, error = %e, "事件处理失败");
                    Err(e)
                }
            }
        } else {
            warn!(event_type = event_type, "未找到事件处理器，跳过");
            Ok(())
        }
    }

    /// 获取已注册的事件类型列表
    pub async fn registered_events(&self) -> Vec<String> {
        let handlers = self.handlers.read().await;
        handlers.keys().cloned().collect()
    }
}

/// 创建飞书事件分发器
///
/// 预注册飞书常用的事件处理器。
pub async fn create_feishu_event_dispatcher() -> EventDispatcher {
    let dispatcher = EventDispatcher::new();

    // 注册消息接收事件
    let handler = |_data: Value| -> HandlerFuture {
        Box::pin(async move {
            info!(event_type = "im.message.receive_v1", "收到消息事件");
            Ok(())
        })
    };
    dispatcher.register("im.message.receive_v1", handler).await;

    // 注册消息已读事件
    let handler = |_data: Value| -> HandlerFuture {
        Box::pin(async move {
            debug!(event_type = "im.message.message_read_v1", "收到已读回执事件");
            Ok(())
        })
    };
    dispatcher.register("im.message.message_read_v1", handler).await;

    // 注册机器人加入群聊事件
    let handler = |_data: Value| -> HandlerFuture {
        Box::pin(async move {
            info!(event_type = "im.chat.member.bot.added_v1", "机器人被添加到群聊");
            Ok(())
        })
    };
    dispatcher.register("im.chat.member.bot.added_v1", handler).await;

    // 注册机器人离开群聊事件
    let handler = |_data: Value| -> HandlerFuture {
        Box::pin(async move {
            info!(event_type = "im.chat.member.bot.deleted_v1", "机器人被移出群聊");
            Ok(())
        })
    };
    dispatcher.register("im.chat.member.bot.deleted_v1", handler).await;

    info!("飞书事件分发器已创建，已注册基础事件处理器");

    dispatcher
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_event_dispatcher_register_and_dispatch() {
        let dispatcher = EventDispatcher::new();
        let mut called = false;

        let handler = |_data: Value| -> HandlerFuture {
            Box::pin(async move {
                called = true;
                Ok(())
            })
        };
        dispatcher.register("test.event", handler).await;

        let result = dispatcher.dispatch("test.event", serde_json::json!({"key": "value"})).await;

        assert!(result.is_ok());
        assert!(called);
    }

    #[tokio::test]
    async fn test_event_dispatcher_unknown_event() {
        let dispatcher = EventDispatcher::new();

        let result = dispatcher.dispatch("unknown.event", serde_json::json!({})).await;

        assert!(result.is_ok()); // 未知事件不应该报错
    }

    #[tokio::test]
    async fn test_event_dispatcher_registered_events() {
        let dispatcher = EventDispatcher::new();

        let handler = |_data: Value| -> HandlerFuture {
            Box::pin(async { Ok(()) })
        };
        dispatcher.register("event.1", handler).await;
        let handler = |_data: Value| -> HandlerFuture {
            Box::pin(async { Ok(()) })
        };
        dispatcher.register("event.2", handler).await;

        let events = dispatcher.registered_events().await;

        assert_eq!(events.len(), 2);
        assert!(events.contains(&"event.1".to_string()));
        assert!(events.contains(&"event.2".to_string()));
    }
}
