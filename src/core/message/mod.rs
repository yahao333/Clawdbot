//! 消息处理模块
//!
//! 本模块负责：
//! 1. 定义消息类型（入站消息、出站消息）
//! 2. 实现消息队列（带去重功能）
//! 3. 实现消息处理器
//!
//! # 消息处理流程
//! ```
//! 收到消息 → 去重检查 → 加入队列 → 路由到 Agent → AI 处理 → 发送响应
//! ```

pub mod types;      // 消息类型定义
pub mod queue;      // 消息队列
pub mod handler;    // 消息处理器
pub mod sender;     // 消息发送器

// 重新导出常用类型
pub use types::{InboundMessage, OutboundMessage, MessageContent, MessageSource, SenderInfo, TargetInfo};
pub use queue::{MessageQueue, QueueConfig, QueueMode, QueueDropPolicy, QueueError};
pub use handler::{HandlerContext, MessageHandler, DefaultMessageHandler, HandlerResult};
pub use sender::{MessageSender, UnifiedMessageSender};
