//! Rust Clawdbot 库入口
//!
//! 本模块导出所有公共 API。
//!
//! # 使用示例
//! ```rust
//! use clawdbot::Config;
//! ```

/// 重新导出核心模块
pub mod cli;
pub mod core;
pub mod channels;
pub mod ai;
// routing 已在 core 中实现，不需要独立模块
pub mod media;
pub mod infra;
pub mod extensions;
pub mod utils;
pub mod service;
pub mod web;
/// 安全审计模块
pub mod security;

/// Sandbox 沙箱模块
///
/// 提供安全的文件操作沙箱功能，用户只能访问其专属目录下的文件
pub mod sandbox;
