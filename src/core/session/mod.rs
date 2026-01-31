//! 会话管理模块
//!
//! 本模块负责管理对话会话，提供会话的创建、查询、历史管理和去重功能。
//!
//! # 模块结构
//! - `types` - 会话类型定义
//! - `store` - 会话存储实现
//! - `sqlite_store` - SQLite 会话存储实现
//! - `manager` - 会话管理器
//!
//! # 使用示例
//! ```rust
//! use crate::core::session::{SessionManager, SessionConfig};
//!
//! let config = SessionConfig::default();
//! let manager = SessionManager::new(Some(config));
//! ```

pub mod types;
pub mod store;
pub mod sqlite_store;
pub mod manager;

pub use types::{Session, SessionMessage, SessionConfig, SessionStatus, SessionQuery, SessionStats};
pub use store::{SessionStore, InMemorySessionStore, SessionStoreConfig};
pub use sqlite_store::SqliteSessionStore;
pub use manager::SessionManager;
