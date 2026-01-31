//! 消息处理器模块
//!
//! 本模块实现了消息处理的核心逻辑。
//!
//! # 处理流程
//! ```
//! 1. 接收入站消息
//! 2. 入站去重检查（避免重复处理）
//! 3. 消息队列处理（支持去重和防抖）
//! 4. 路由到对应的 Agent
//! 5. AI 执行生成响应
//! 6. 发送响应消息
//! ```
//!
//! # 关键设计
//! - 使用 `async-trait` 实现异步 trait
//! - 支持消息处理前后的钩子函数
//! - 依赖注入模式，便于测试

use std::sync::Arc;
use tracing::{debug, info, instrument, warn};

use super::{types::InboundMessage, types::OutboundMessage, queue::{MessageQueue, QueueError}, sender::MessageSender};
use crate::core::routing::Router;
use crate::core::agent::AiEngine;
use crate::core::session::SessionManager;
use crate::infra::error::Result;

/// 消息处理上下文
///
/// 包含消息处理所需的所有依赖
///
/// # 字段说明
/// * `config` - 配置（用于获取各种参数）
/// * `queue` - 消息队列
/// * `router` - 路由器（用于路由消息到 Agent）
/// * `ai_engine` - AI 引擎（用于生成响应）
/// * `sender` - 消息发送器（用于发送响应）
/// * `session_manager` - 会话管理器（用于会话管理和消息去重）
#[derive(Clone)]
pub struct HandlerContext {
    /// 配置
    config: Arc<super::super::super::infra::config::Config>,
    /// 消息队列
    queue: Arc<MessageQueue>,
    /// 路由器
    router: Arc<dyn Router>,
    /// AI 引擎（使用 trait object 支持不同实现）
    ai_engine: Arc<dyn AiEngine>,
    /// 消息发送器
    sender: Arc<dyn MessageSender>,
    /// 会话管理器
    session_manager: Arc<SessionManager>,
}

impl HandlerContext {
    /// 创建消息处理上下文
    ///
    /// # 参数说明
    /// * `config` - 配置
    /// * `queue` - 消息队列
    /// * `router` - 路由器
    /// * `ai_engine` - AI 引擎
    /// * `sender` - 消息发送器
    /// * `session_manager` - 会话管理器
    ///
    /// # 返回值
    /// 创建的上下文
    pub fn new(
        config: Arc<super::super::super::infra::config::Config>,
        queue: Arc<MessageQueue>,
        router: Arc<dyn Router>,
        ai_engine: Arc<dyn AiEngine>,
        sender: Arc<dyn MessageSender>,
        session_manager: Arc<SessionManager>,
    ) -> Self {
        Self {
            config,
            queue,
            router,
            ai_engine,
            sender,
            session_manager,
        }
    }
}

/// 消息处理器 Trait
///
/// 定义消息处理器的接口
///
/// # 实现要求
/// - 必须实现 `Send + Sync` 以支持多线程
/// - 所有方法必须是异步的
///
/// # 示例
/// ```rust
/// #[async_trait::async_trait]
/// impl MessageHandler for MyHandler {
///     async fn handle(&self, ctx: &HandlerContext, message: InboundMessage) -> Result<()> {
///         // 处理消息
///         Ok(())
///     }
/// }
/// ```
#[async_trait::async_trait]
pub trait MessageHandler: Send + Sync {
    /// 处理入站消息
    ///
    /// # 参数说明
    /// * `ctx` - 消息处理上下文
    /// * `message` - 入站消息
    ///
    /// # 返回值
    /// 处理成功返回 `Ok(())`，失败返回错误
    async fn handle(&self, ctx: &HandlerContext, message: InboundMessage) -> Result<()>;

    /// 处理消息前的钩子
    ///
    /// 在消息处理前调用，可用于：
    /// - 消息验证
    /// - 权限检查
    /// - 日志记录
    ///
    /// # 参数说明
    /// * `message` - 要处理的消息
    ///
    /// # 返回值
    /// 验证通过返回 `Ok(())`，验证失败返回错误
    async fn before_handle(&self, message: &InboundMessage) -> Result<()> {
        Ok(())
    }

    /// 处理消息后的钩子
    ///
    /// 在消息处理后调用，可用于：
    /// - 清理资源
    /// - 更新统计
    /// - 发送通知
    ///
    /// # 参数说明
    /// * `message` - 已处理的消息
    /// * `result` - 处理结果
    async fn after_handle(&self, message: &InboundMessage, result: &Result<()>) {
        // 默认实现为空
    }
}

/// 默认消息处理器
///
/// 标准的消息处理器实现
///
/// # 处理步骤
/// 1. 入站去重检查
/// 2. 消息队列处理
/// 3. 路由到对应 Agent
/// 4. AI 执行生成响应
/// 5. 发送响应
#[derive(Debug, Clone, Default)]
pub struct DefaultMessageHandler {
    // 默认处理器不需要额外字段
}

#[async_trait::async_trait]
impl MessageHandler for DefaultMessageHandler {
    /// 处理入站消息
    ///
    /// # 处理流程
    /// 1. 调用 `before_handle` 钩子
    /// 2. 入站去重检查
    /// 3. 消息队列处理
    /// 4. 路由到 Agent
    /// 5. AI 执行
    /// 6. 发送响应
    /// 7. 调用 `after_handle` 钩子
    ///
    /// # 参数说明
    /// * `ctx` - 消息处理上下文
    /// * `message` - 入站消息
    ///
    /// # 返回值
    /// 处理成功返回 `Ok(())`
    ///
    /// # 日志记录
    /// - INFO: 开始处理
    /// - DEBUG: 处理步骤
    /// - WARN: 重复消息
    /// - ERROR: 处理失败
    #[instrument(skip(ctx, message), fields(message_id = %message.id, channel = %message.channel))]
    async fn handle(&self, ctx: &HandlerContext, message: InboundMessage) -> Result<()> {
        let message_id = message.id.clone();

        // 1. 调用处理前钩子
        debug!("开始处理消息前钩子");
        self.before_handle(&message).await?;

        // 2. 入站去重检查
        debug!("执行入站去重检查");
        if ctx.queue.is_duplicate(&message).await {
            warn!(message_id = %message.id, "消息重复，跳过处理");
            return Ok(());
        }

        // 3. 消息队列处理（去重和防抖）
        debug!("消息入队处理");
        match ctx.queue.push(message.clone()).await {
            Ok(_) => {
                info!(message_id = %message_id, "消息已入队待处理");
            }
            Err(QueueError::Duplicate { message_id }) => {
                warn!(message_id = %message_id, "消息重复，跳过处理");
                return Ok(());
            }
            Err(e) => {
                return Err(crate::infra::error::Error::Channel(e.to_string()));
            }
        }

        // 7. 调用处理后钩子（表示已成功入队）
        self.after_handle(&message, &Ok(())).await;

        Ok(())
    }
}

impl DefaultMessageHandler {
    /// 核心消息处理逻辑（由队列消费者调用）
    pub async fn process_message(ctx: HandlerContext, message: InboundMessage) {
        let message_id = message.id.clone();
        debug!(message_id = %message_id, "开始从队列消费并处理消息");

        // 获取会话
        let route_match = match ctx.router.route(&message).await {
            Ok(r) => r,
            Err(e) => {
                tracing::error!(error = %e, "消息路由失败");
                return;
            }
        };

        // 会话去重检查
        let session = ctx.session_manager.get_or_create_session(&message, &route_match.agent_id).await;
        if ctx.session_manager.is_message_processed(&message_id, &session.id).await {
            info!(message_id = %message_id, "消息已处理过，跳过");
            return;
        }

        info!(
            agent_id = %route_match.agent_id,
            confidence = route_match.confidence,
            session_id = %session.id,
            "消息路由成功"
        );

        // 提取消息内容用于历史记录
        let message_content = message.content.text.clone()
            .or(message.content.rich_text.as_ref().map(|r| r.content.clone()))
            .unwrap_or_default();

        // 添加用户消息到会话历史
        ctx.session_manager.add_message_to_session(&session.id, "user", &message_content).await;

        // 5. AI 执行生成响应
        debug!("开始 AI 执行");
        let response = match ctx.ai_engine.execute(&route_match.agent_id, &message).await {
             Ok(r) => r,
             Err(e) => {
                 tracing::error!(error = %e, "AI 执行失败");
                 return;
             }
        };

        // 添加助手响应到会话历史
        if let Some(ref text) = response.text {
            ctx.session_manager.add_message_to_session(&session.id, "assistant", text).await;
        }

        // 6. 发送响应
        debug!("准备发送响应");
        if let Some(resp_content) = response.text {
             let outbound = OutboundMessage {
                target_id: message.target.id.clone(), // 回复给同一个目标
                channel: message.channel.clone(),     // 使用相同的渠道
                content: super::types::MessageContent::text(&resp_content),
                thread_id: None,
                enable_push: Some(true),
                card: None,
            };

            match ctx.sender.send(outbound).await {
                Ok(msg_id) => info!(response_id = %msg_id, "响应发送成功"),
                Err(e) => tracing::error!(error = %e, "响应发送失败"),
            }
        }

        info!(message_id = %message_id, session_id = %session.id, "消息处理完成");
    }
}

/// 消息处理结果
///
/// 记录消息处理的结果信息
#[derive(Debug, Clone)]
pub struct HandlerResult {
    /// 消息 ID
    pub message_id: String,
    /// 是否成功
    pub success: bool,
    /// 错误信息（如果失败）
    pub error: Option<String>,
    /// 处理耗时（毫秒）
    pub duration_ms: u64,
}

impl HandlerResult {
    /// 创建成功的结果
    pub fn success(message_id: &str, duration_ms: u64) -> Self {
        Self {
            message_id: message_id.to_string(),
            success: true,
            error: None,
            duration_ms,
        }
    }

    /// 创建失败的结果
    pub fn failure(message_id: &str, error: &str, duration_ms: u64) -> Self {
        Self {
            message_id: message_id.to_string(),
            success: false,
            error: Some(error.to_string()),
            duration_ms,
        }
    }
}
