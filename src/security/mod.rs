//! 安全审计模块
//!
//! 提供完整的安全审计功能
//!
//! # 模块结构
//! - [`audit`] - 审计服务主入口
//! - [`types`] - 类型定义
//! - [`storage`] - 存储后端
//! - [`alert`] - 告警服务

pub mod audit;
pub mod types;
pub mod storage;
pub mod alert;

pub use audit::AuditService;
pub use types::{AuditConfig, AuditEvent, AuditLevel, AuditCategory, AuditContext, AuditQuery, AuditStats};
