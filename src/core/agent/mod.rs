//! Agent 执行器模块
//!
//! 本模块负责：
//! 1. 管理 Agent 的配置和状态
//! 2. 调用 AI 模型生成响应
//! 3. 管理执行上下文和会话历史

pub mod executor;
pub mod context;

// 重新导出常用类型
pub use executor::{AiEngine, AiResponse, DefaultAiEngine};
pub use context::{AgentContext, AgentConfig, AgentStore, MessageRole, ChatMessage};
