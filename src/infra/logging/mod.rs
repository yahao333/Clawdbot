//! 日志系统模块
//!
//! 本模块提供了统一的日志记录功能，使用 `tracing` 库实现。

use std::path::PathBuf;
use tracing::{info, Level};
use tracing_subscriber::fmt;

/// 日志级别
///
/// 从低到高：Trace < Debug < Info < Warn < Error
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    /// 最详细的日志级别（调试用）
    Trace,
    /// 调试信息
    Debug,
    /// 一般信息
    Info,
    /// 警告
    Warn,
    /// 错误
    Error,
}

/// 日志格式
///
/// 日志的输出格式
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogFormat {
    /// 默认格式（人类可读）
    Default,
    /// JSON 格式（机器可读）
    Json,
}

/// 日志配置
///
/// 配置日志系统的行为
#[derive(Debug, Clone)]
pub struct LoggingConfig {
    /// 日志级别
    pub level: LogLevel,
    /// 日志格式
    pub format: LogFormat,
    /// 是否输出到文件
    pub file: bool,
    /// 日志文件路径
    pub file_path: PathBuf,
}

impl Default for LoggingConfig {
    /// 默认配置
    fn default() -> Self {
        Self {
            level: LogLevel::Info,
            format: LogFormat::Default,
            file: false,
            file_path: PathBuf::from("clawdbot.log"),
        }
    }
}

/// 初始化日志系统
///
/// # 参数说明
/// * `config` - 日志配置
pub fn init(config: &LoggingConfig) {
    let level_filter = match config.level {
        LogLevel::Trace => Level::TRACE,
        LogLevel::Debug => Level::DEBUG,
        LogLevel::Info => Level::INFO,
        LogLevel::Warn => Level::WARN,
        LogLevel::Error => Level::ERROR,
    };

    let subscriber = tracing_subscriber::fmt()
        .with_max_level(level_filter)
        .finish();

    tracing::subscriber::set_global_default(subscriber)
        .expect("设置全局日志 subscriber 失败");

    info!(level = ?config.level, "日志系统初始化完成");
}
