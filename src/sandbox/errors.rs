//! Sandbox 模块错误定义
//!
//! 定义沙箱环境相关的错误类型，使用 thiserror 进行错误定义
//!
//! # 使用示例
//! ```rust
//! use clawdbot::sandbox::SandboxError;
//!
//! match result {
//!     Ok(_) => println!("操作成功"),
//!     Err(SandboxError::PathTraversal { path }) => {
//!         println!("检测到路径遍历攻击: {:?}", path);
//!     }
//!     Err(e) => println!("其他错误: {}", e),
//! }
//! ```

use thiserror::Error;

/// 沙箱错误类型
///
/// 包含沙箱操作过程中可能出现的所有错误类型
#[derive(Error, Debug)]
pub enum SandboxError {
    /// 沙箱功能未启用
    ///
    /// 当沙箱功能被禁用时，所有沙箱操作都会返回此错误
    #[error("沙箱功能未启用")]
    Disabled,

    /// 路径遍历攻击检测
    ///
    /// 当请求的路径尝试访问沙箱外的资源时返回此错误
    #[error("路径遍历攻击: {:?}", .path)]
    PathTraversal {
        /// 被攻击的路径
        path: std::path::PathBuf,
    },

    /// 用户沙箱目录不存在
    ///
    /// 当用户沙箱目录被删除或尚未创建时返回此错误
    #[error("用户沙箱目录不存在: user_id={}", .user_id)]
    UserSandboxNotFound {
        /// 用户 ID
        user_id: String,
    },

    /// 超出磁盘配额
    ///
    /// 当写入操作会导致超出用户磁盘配额时返回此错误
    #[error("超出磁盘配额: 当前={} 字节, 限制={} 字节", .current_size, .max_size)]
    QuotaExceeded {
        /// 当前已使用空间（字节）
        current_size: u64,
        /// 最大配额（字节）
        max_size: u64,
    },

    /// 磁盘空间不足
    ///
    /// 当写入操作所需的磁盘空间不足时返回此错误
    #[error("磁盘空间不足: 需要={} 字节, 可用={} 字节", .required, .available)]
    InsufficientSpace {
        /// 需要的空间（字节）
        required: u64,
        /// 可用空间（字节）
        available: u64,
    },

    /// 文件不存在
    ///
    /// 当请求的文件或目录不存在时返回此错误
    #[error("文件不存在: {:?}", .path)]
    NotFound {
        /// 请求的路径
        path: std::path::PathBuf,
    },

    /// 无权限访问
    ///
    /// 当用户没有权限访问指定资源时返回此错误
    #[error("无权限访问: {:?}", .path)]
    PermissionDenied {
        /// 请求的路径
        path: std::path::PathBuf,
    },

    /// 符号链接循环
    ///
    /// 当检测到符号链接循环时返回此错误
    #[error("符号链接循环: {:?}", .path)]
    SymlinkLoop {
        /// 涉及的路径
        path: std::path::PathBuf,
    },

    /// 符号链接被禁用
    ///
    /// 当请求创建或跟随符号链接，但符号链接功能被禁用时返回此错误
    #[error("符号链接功能已禁用")]
    SymlinksDisabled,

    /// 文件操作错误
    ///
    /// 底层文件系统操作失败
    #[error("文件操作错误: {:?}", .source)]
    IoError {
        /// 原始 IO 错误
        #[from]
        source: std::io::Error,
    },

    /// 用户目录创建失败
    ///
    /// 初始化用户沙箱目录结构时失败
    #[error("创建用户沙箱目录失败: {:?}", .path)]
    UserDirCreationFailed {
        /// 尝试创建的路径
        path: std::path::PathBuf,
        /// 原始错误
        source: std::io::Error,
    },
}

impl SandboxError {
    /// 检查是否为安全相关错误
    ///
    /// 安全相关错误包括路径遍历、权限拒绝等
    ///
    /// # 返回值
    /// 如果是安全相关错误返回 true
    pub fn is_security_error(&self) -> bool {
        matches!(
            self,
            SandboxError::PathTraversal { .. }
                | SandboxError::PermissionDenied { .. }
                | SandboxError::SymlinksDisabled
                | SandboxError::SymlinkLoop { .. }
        )
    }

    /// 获取错误的安全等级
    ///
    /// 用于日志记录和告警
    ///
    /// # 返回值
    /// 错误的安全等级字符串
    pub fn security_level(&self) -> &'static str {
        match self {
            SandboxError::PathTraversal { .. } => "HIGH",
            SandboxError::PermissionDenied { .. } => "MEDIUM",
            SandboxError::SymlinkLoop { .. } => "HIGH",
            SandboxError::SymlinksDisabled => "LOW",
            _ => "INFO",
        }
    }
}

/// 沙箱操作结果类型
///
/// 统一所有沙箱操作的返回类型
pub type SandboxResult<T> = Result<T, SandboxError>;
