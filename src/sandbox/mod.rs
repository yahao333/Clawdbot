//! Sandbox 模块
//!
//! 提供安全的文件操作沙箱功能，用户只能访问其专属目录下的文件
//!
//! # 主要功能
//! - **路径隔离**：用户只能访问沙箱目录内的文件
//! - **磁盘配额**：限制用户的磁盘使用量
//! - **安全验证**：防止路径遍历和符号链接攻击
//! - **文件操作**：支持读写、列出、创建、删除等操作
//!
//! # 目录结构
//! ```
//! data/sandbox/
//!   ├── user_{user_id}/
//!   │   ├── files/        # 用户文件目录
//!   │   ├── uploads/      # 上传文件目录
//!   │   └── downloads/    # 下载文件目录
//!   └── .gitkeep
//! ```
//!
//! # 使用示例
//! ```rust
//! use clawdbot::sandbox::{SandboxService, SandboxConfig};
//! use std::sync::Arc;
//!
//! // 创建沙箱服务
//! let service = SandboxService::new("./data/sandbox").await?;
//!
//! // 读取文件
//! let content = service.read_file("user123", "documents/test.txt", None, None).await?;
//!
//! // 写入文件
//! service.write_file("user123", "documents/test.txt", b"Hello").await?;
//!
//! // 列出目录
//! let entries = service.list_dir("user123", "documents").await?;
//! ```
//!
//! # 配置项
//! ```toml
//! [sandbox]
//! enabled = true
//! base_path = "data/sandbox"
//! max_size_mb = 100
//! allow_symlinks = false
//! ```

// 模块子模块
pub mod types;          // 类型定义
pub mod errors;         // 错误定义
pub mod path;           // 路径验证工具
pub mod service;        // 沙箱服务

// 重新导出主要类型
pub use types::{
    SandboxConfig,
    UserSandbox,
    SandboxEntry,
    SandboxOperation,
};

pub use errors::{
    SandboxError,
    SandboxResult,
};

pub use path::PathValidator;

pub use service::SandboxService;
