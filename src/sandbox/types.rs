//! Sandbox 模块类型定义
//!
//! 提供沙箱环境相关的类型定义，包括配置、用户沙箱信息等
//!
//! # 使用示例
//! ```rust
//! use clawdbot::sandbox::SandboxConfig;
//!
//! let config = SandboxConfig::default();
//! println!("沙箱根目录: {:?}", config.base_path);
//! ```

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// 沙箱配置
///
/// 配置沙箱环境的基本参数，包括根目录、是否启用等
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxConfig {
    /// 是否启用沙箱功能
    ///
    /// 设置为 false 时，所有文件操作将绕过沙箱验证
    #[serde(default = "default_enabled")]
    pub enabled: bool,

    /// 沙箱根目录路径
    ///
    /// 所有用户沙箱目录的父目录，默认为 `data/sandbox`
    #[serde(default = "default_base_path")]
    pub base_path: PathBuf,

    /// 用户子目录名称格式
    ///
    /// 用于格式化用户目录名称，支持 `{user_id}` 占位符
    #[serde(default = "default_user_dir_format")]
    pub user_dir_format: String,

    /// 最大磁盘使用配额（MB）
    ///
    /// 单个用户沙箱目录的最大磁盘使用量，0 表示无限制
    #[serde(default = "default_max_size_mb")]
    pub max_size_mb: u64,

    /// 是否允许符号链接
    ///
    /// 如果为 true，将验证符号链接指向的目标是否在沙箱内
    #[serde(default = "default_allow_symlinks")]
    pub allow_symlinks: bool,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            base_path: default_base_path(),
            user_dir_format: default_user_dir_format(),
            max_size_mb: default_max_size_mb(),
            allow_symlinks: default_allow_symlinks(),
        }
    }
}

/// 默认启用状态
fn default_enabled() -> bool {
    true
}

/// 默认沙箱根目录
fn default_base_path() -> PathBuf {
    PathBuf::from("data/sandbox")
}

/// 默认用户目录格式
fn default_user_dir_format() -> String {
    "user_{user_id}".to_string()
}

/// 默认最大磁盘配额（100MB）
fn default_max_size_mb() -> u64 {
    100
}

/// 默认是否允许符号链接
fn default_allow_symlinks() -> bool {
    false
}

/// 用户沙箱信息
///
/// 包含单个用户的沙箱目录信息和统计
#[derive(Debug, Clone)]
pub struct UserSandbox {
    /// 用户唯一标识
    pub user_id: String,

    /// 用户沙箱根目录
    pub root_path: PathBuf,

    /// 文件子目录
    pub files_path: PathBuf,

    /// 上传子目录
    pub uploads_path: PathBuf,

    /// 下载子目录
    pub downloads_path: PathBuf,

    /// 当前磁盘使用量（字节）
    pub current_size: u64,

    /// 最大磁盘配额（字节）
    pub max_size: u64,
}

impl UserSandbox {
    /// 创建用户沙箱
    ///
    /// # 参数说明
    /// * `user_id` - 用户唯一标识
    /// * `root_path` - 沙箱根目录
    /// * `max_size` - 最大磁盘配额（字节）
    ///
    /// # 返回值
    /// 创建的用户沙箱信息
    pub fn new(user_id: &str, root_path: PathBuf, max_size: u64) -> Self {
        let files_path = root_path.join("files");
        let uploads_path = root_path.join("uploads");
        let downloads_path = root_path.join("downloads");

        Self {
            user_id: user_id.to_string(),
            root_path,
            files_path,
            uploads_path,
            downloads_path,
            current_size: 0,
            max_size,
        }
    }

    /// 检查是否超出配额
    ///
    /// # 返回值
    /// 如果超出配额返回 true，否则返回 false
    pub fn is_quota_exceeded(&self, additional_bytes: u64) -> bool {
        if self.max_size == 0 {
            return false; // 无限制
        }
        self.current_size + additional_bytes > self.max_size
    }

    /// 获取剩余可用空间
    ///
    /// # 返回值
    /// 剩余可用空间（字节），0 表示无限制
    pub fn available_space(&self) -> u64 {
        if self.max_size == 0 {
            u64::MAX
        } else if self.current_size >= self.max_size {
            0
        } else {
            self.max_size - self.current_size
        }
    }
}

/// 文件/目录条目信息
///
/// 用于列出目录内容时返回的条目信息
#[derive(Debug, Clone)]
pub struct SandboxEntry {
    /// 条目名称
    pub name: String,

    /// 条目完整路径
    pub path: PathBuf,

    /// 是否为目录
    pub is_dir: bool,

    /// 文件大小（字节），目录为 0
    pub size: u64,

    /// 最后修改时间
    pub modified: std::time::SystemTime,
}

impl SandboxEntry {
    /// 从 `tokio::fs::DirEntry` 创建条目
    ///
    /// # 参数说明
    /// * `dir_entry` - tokio 目录条目
    ///
    /// # 返回值
    /// 创建的条目信息
    ///
    /// # 错误
    /// 如果获取文件元数据失败，返回错误
    pub async fn from_dir_entry(
        dir_entry: &tokio::fs::DirEntry,
    ) -> Result<Self, std::io::Error> {
        let path = dir_entry.path().to_path_buf();
        let metadata = dir_entry.metadata().await?;

        let name = path.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        let size = if metadata.is_file() {
            metadata.len()
        } else {
            0
        };

        Ok(Self {
            name,
            path,
            is_dir: metadata.is_dir(),
            size,
            modified: metadata.modified()?,
        })
    }
}

/// 沙箱操作类型
///
/// 表示可以在沙箱中执行的操作类型
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SandboxOperation {
    /// 读取文件
    Read,
    /// 写入文件
    Write,
    /// 创建目录
    CreateDir,
    /// 列出目录
    List,
    /// 删除文件/目录
    Delete,
    /// 创建符号链接
    Symlink,
}

impl std::fmt::Display for SandboxOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SandboxOperation::Read => write!(f, "读取"),
            SandboxOperation::Write => write!(f, "写入"),
            SandboxOperation::CreateDir => write!(f, "创建目录"),
            SandboxOperation::List => write!(f, "列出目录"),
            SandboxOperation::Delete => write!(f, "删除"),
            SandboxOperation::Symlink => write!(f, "创建符号链接"),
        }
    }
}
