//! 飞书渠道适配器模块
//!
//! 本模块实现了飞书平台的渠道适配器。
//!
//! # 功能
//! - 接收消息（通过 WebSocket 长链接）
//! - 发送消息（通过 API）
//! - 消息处理
//!
//! # 配置文件示例
//! ```toml
//! [channels.feishu]
//! channel_type = "feishu"
//! enabled = true
//! [channels.feishu.credentials]
//! app_id = "${FEISHU_APP_ID}"
//! app_secret = "${FEISHU_APP_SECRET}"
//! verification_token = "${FEISHU_VERIFICATION_TOKEN}"
//! ```

pub mod client;      // HTTP 客户端
pub mod handlers;    // 事件处理
pub mod send;        // 消息发送
pub mod monitor;     // 消息监控（轮询）
pub mod ws;          // WebSocket 长链接

// 重新导出常用类型
pub use client::{FeishuClient, FeishuCredentials};
pub use handlers::{MessageEventHandler, FeishuEventRequest, MessageReceiveResult};
pub use send::{FeishuMessageSender, MessageReceiver, SendMessageResponse};
pub use monitor::FeishuMessageMonitor;
pub use ws::FeishuWsMonitor;
