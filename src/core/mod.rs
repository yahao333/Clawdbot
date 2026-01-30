//! 核心运行时模块
//!
//! 机器人的"大脑"，负责消息处理、路由、AI 执行、会话管理等核心功能
//!
//! # 模块结构
//! - `message/` - 消息处理（接收、队列、处理器）
//! - `routing/` - 消息路由（决定消息由哪个 Agent 处理）
//! - `agent/` - Agent 执行（调用 AI 模型）
//! - `session/` - 会话管理（对话历史和上下文）

pub mod message;
pub mod routing;
pub mod agent;
pub mod session;
