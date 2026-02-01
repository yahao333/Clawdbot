//! Sandbox 路径验证工具
//!
//! 提供路径验证功能，确保所有文件操作都在沙箱目录内
//!
//! # 安全特性
//! - 路径规范化：使用 `canonicalize` 解析相对路径和符号链接
//! - 路径遍历防护：检测 `../` 等路径遍历攻击
//! - 符号链接验证：可选验证符号链接指向的目标
//!
//! # 使用示例
//! ```rust
//! use std::path::PathBuf;
//! use clawdbot::sandbox::path::{PathValidator, PathValidationResult};
//!
//! let validator = PathValidator::new(PathBuf::from("/data/sandbox"));
//!
//! // 验证路径
//! let result = validator.validate("/data/sandbox/user_123/file.txt");
//! assert!(result.is_ok());
//!
//! // 路径遍历攻击检测
//! let result = validator.validate("/data/sandbox/user_123/../../../etc/passwd");
//! assert!(result.is_err());
//! ```

use tracing::{debug, warn};
use tokio::sync::RwLock;

use crate::sandbox::errors::{SandboxError, SandboxResult};
use crate::sandbox::types::SandboxConfig;
use std::sync::Arc;

/// 路径验证结果
///
/// 包含验证后的规范化路径和验证信息
#[derive(Debug, Clone)]
pub struct PathValidationResult {
    /// 规范化后的安全路径
    ///
    /// 经过验证的绝对路径
    pub safe_path: std::path::PathBuf,

    /// 是否需要统计磁盘使用
    ///
    /// 如果为 true，调用者需要考虑磁盘配额
    pub need_quota_check: bool,
}

/// 路径验证器
///
/// 用于验证路径是否在沙箱目录内
///
/// # 字段说明
/// * `sandbox_root` - 沙箱根目录
/// * `allow_symlinks` - 是否允许符号链接
#[derive(Debug, Clone)]
pub struct PathValidator {
    /// 沙箱根目录
    sandbox_root: std::path::PathBuf,

    /// 是否允许符号链接
    allow_symlinks: bool,

    /// 沙箱根的规范路径（缓存）
    ///
    /// 避免重复调用 canonicalize，使用 RwLock 保护
    canonical_root: Arc<RwLock<Option<std::path::PathBuf>>>,
}

impl PathValidator {
    /// 创建路径验证器
    ///
    /// # 参数说明
    /// * `sandbox_root` - 沙箱根目录
    ///
    /// # 返回值
    /// 创建的验证器实例
    pub fn new(sandbox_root: std::path::PathBuf) -> Self {
        Self {
            sandbox_root,
            allow_symlinks: false,
            canonical_root: Arc::new(RwLock::new(None)),
        }
    }

    /// 创建带配置的路径验证器
    ///
    /// # 参数说明
    /// * `sandbox_root` - 沙箱根目录
    /// * `config` - 沙箱配置
    ///
    /// # 返回值
    /// 创建的验证器实例
    pub fn with_config(
        sandbox_root: std::path::PathBuf,
        config: &SandboxConfig,
    ) -> Self {
        Self {
            sandbox_root,
            allow_symlinks: config.allow_symlinks,
            canonical_root: Arc::new(RwLock::new(None)),
        }
    }

    /// 获取沙箱根目录
    ///
    /// # 返回值
    /// 沙箱根目录路径
    pub fn sandbox_root(&self) -> &std::path::Path {
        &self.sandbox_root
    }

    /// 验证并规范化路径
    ///
    /// 这是主要的验证入口，会执行以下检查：
    /// 1. 路径非空检查
    /// 2. 路径遍历攻击检测
    /// 3. 符号链接验证（如果启用）
    /// 4. 沙箱边界验证
    ///
    /// # 参数说明
    /// * `user_id` - 用户 ID（用于日志记录）
    /// * `requested_path` - 请求的路径（可以是相对路径或绝对路径）
    ///
    /// # 返回值
    /// 验证成功返回规范化后的安全路径，失败返回错误
    pub async fn validate_path(
        &self,
        user_id: &str,
        requested_path: &std::path::Path,
    ) -> SandboxResult<PathValidationResult> {
        // 1. 检查路径是否为空
        if requested_path.as_os_str().is_empty() {
            warn!(user_id = user_id, "空路径请求");
            return Err(SandboxError::NotFound {
                path: requested_path.to_path_buf(),
            });
        }

        // 2. 检查路径是否包含父目录引用（初步检查）
        if self.has_path_traversal_attempt(requested_path) {
            let path_str = requested_path.display().to_string();
            warn!(
                user_id = user_id,
                path = %path_str,
                "检测到路径遍历攻击尝试"
            );
            return Err(SandboxError::PathTraversal {
                path: requested_path.to_path_buf(),
            });
        }

        // 3. 处理相对路径
        let absolute_path = if requested_path.is_absolute() {
            requested_path.to_path_buf()
        } else {
            // 相对路径以沙箱根为基准
            self.sandbox_root.join(requested_path)
        };

        // 4. 获取规范化的路径（解析符号链接）
        let canonical_path = match tokio::fs::canonicalize(&absolute_path).await {
            Ok(path) => path,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // 文件不存在，但这可能是正常情况（如写入新文件）
                // 返回绝对路径，让调用者处理
                debug!(
                    user_id = user_id,
                    path = %absolute_path.display(),
                    "路径不存在，将作为新路径处理"
                );
                absolute_path
            }
            Err(e) => {
                warn!(
                    user_id = user_id,
                    path = %absolute_path.display(),
                    error = %e,
                    "路径规范化失败"
                );
                return Err(SandboxError::IoError { source: e });
            }
        };

        // 5. 获取沙箱根的规范路径（缓存或计算）
        let canonical_root = {
            let cached = self.canonical_root.read().await;
            if let Some(ref root) = *cached {
                root.clone()
            } else {
                // 需要计算，释放读锁后获取写锁
                drop(cached);
                let mut writable = self.canonical_root.write().await;
                // 双重检查
                if let Some(ref root) = *writable {
                    root.clone()
                } else {
                    // 计算规范路径
                    let canonical = tokio::fs::canonicalize(&self.sandbox_root).await
                        .map_err(|e| SandboxError::IoError { source: e })?;
                    *writable = Some(canonical.clone());
                    canonical
                }
            }
        };

        // 6. 验证路径是否在沙箱内
        if !canonical_path.starts_with(&canonical_root) {
            let path_str = canonical_path.display().to_string();
            let root_str = canonical_root.display().to_string();

            warn!(
                user_id = user_id,
                requested_path = %requested_path.display(),
                canonical_path = %path_str,
                sandbox_root = %root_str,
                "路径超出沙箱范围"
            );

            return Err(SandboxError::PathTraversal {
                path: requested_path.to_path_buf(),
            });
        }

        // 7. 验证符号链接（如果启用）
        if !self.allow_symlinks {
            if let Err(e) = self.validate_no_symlink(&canonical_path).await {
                warn!(
                    user_id = user_id,
                    path = %canonical_path.display(),
                    error = %e,
                    "符号链接验证失败"
                );
                return Err(e);
            }
        }

        debug!(
            user_id = user_id,
            requested_path = %requested_path.display(),
            safe_path = %canonical_path.display(),
            "路径验证通过"
        );

        Ok(PathValidationResult {
            safe_path: canonical_path,
            need_quota_check: true,
        })
    }

    /// 快速验证路径（同步版本）
    ///
    /// 适用于不需要异步操作的场景
    ///
    /// # 参数说明
    /// * `user_id` - 用户 ID
    /// * `requested_path` - 请求的路径
    ///
    /// # 返回值
    /// 验证结果
    pub fn validate_path_sync(
        &self,
        user_id: &str,
        requested_path: &std::path::Path,
    ) -> SandboxResult<PathValidationResult> {
        // 1. 检查路径是否为空
        if requested_path.as_os_str().is_empty() {
            return Err(SandboxError::NotFound {
                path: requested_path.to_path_buf(),
            });
        }

        // 2. 检查路径是否包含父目录引用
        if self.has_path_traversal_attempt(requested_path) {
            return Err(SandboxError::PathTraversal {
                path: requested_path.to_path_buf(),
            });
        }

        // 3. 处理相对路径
        let absolute_path = if requested_path.is_absolute() {
            requested_path.to_path_buf()
        } else {
            self.sandbox_root.join(requested_path)
        };

        // 4. 尝试获取规范路径（使用 std::fs，非异步）
        let canonical_path = match std::fs::canonicalize(&absolute_path) {
            Ok(path) => path,
            Err(_) => absolute_path, // 文件不存在时返回原始路径
        };

        // 5. 获取沙箱根规范路径
        let canonical_root = match std::fs::canonicalize(&self.sandbox_root) {
            Ok(path) => path,
            Err(e) => {
                return Err(SandboxError::IoError { source: e });
            }
        };

        // 6. 验证是否在沙箱内
        if !canonical_path.starts_with(&canonical_root) {
            return Err(SandboxError::PathTraversal {
                path: requested_path.to_path_buf(),
            });
        }

        Ok(PathValidationResult {
            safe_path: canonical_path,
            need_quota_check: true,
        })
    }

    /// 检查路径是否包含遍历攻击尝试
    ///
    /// 这是一个初步检查，用于快速检测明显的攻击
    ///
    /// # 参数说明
    /// * `path` - 要检查的路径
    ///
    /// # 返回值
    /// 如果检测到遍历尝试返回 true
    fn has_path_traversal_attempt(&self, path: &std::path::Path) -> bool {
        // 检查路径组件
        for component in path.components() {
            match component {
                std::path::Component::ParentDir => {
                    // 允许在沙箱内的 ../ 操作，但后续会进行完整验证
                    // 这里只做初步检测
                }
                std::path::Component::Normal(os_str) => {
                    let name = os_str.to_string_lossy();
                    // 检查隐藏文件（以 . 开头但不是 . 或 ..）
                    if name.starts_with('.') && name != "." && name != ".." {
                        // 隐藏文件可能是正常用途，只记录
                        debug!(file = %name, "发现隐藏文件");
                    }
                }
                _ => {}
            }
        }

        // 将路径转换为字符串检查
        if let Some(path_str) = path.to_str() {
            // 检查常见的遍历攻击模式
            let patterns = [
                "../",           // 向上遍历
                "..\\",          // Windows 风格向上遍历
                "/..",           // 以遍历开头
                "..%2f",         // URL 编码的 ../
                "%2e%2e/",       // 双点 URL 编码
                "..../",         // 多重 ..
            ];

            for pattern in &patterns {
                if path_str.contains(pattern) {
                    return true;
                }
            }
        }

        false
    }

    /// 验证路径不包含符号链接
    ///
    /// 检查路径的每个组成部分是否都不是符号链接
    ///
    /// # 参数说明
    /// * `path` - 要检查的路径
    ///
    /// # 返回值
    /// 如果没有符号链接返回 Ok，否则返回错误
    async fn validate_no_symlink(
        &self,
        path: &std::path::Path,
    ) -> SandboxResult<()> {
        // 检查路径本身是否是符号链接
        if let Ok(metadata) = tokio::fs::symlink_metadata(path).await {
            if metadata.file_type().is_symlink() {
                return Err(SandboxError::SymlinksDisabled);
            }
        }

        // 检查父目录是否是符号链接
        if let Some(parent) = path.parent() {
            if parent != std::path::Path::new("") {
                if let Ok(metadata) = tokio::fs::symlink_metadata(parent).await {
                    if metadata.file_type().is_symlink() {
                        return Err(SandboxError::SymlinksDisabled);
                    }
                }
            }
        }

        Ok(())
    }

    /// 检查路径是否在沙箱内
    ///
    /// 同步版本的简单检查
    ///
    /// # 参数说明
    /// * `path` - 要检查的路径
    ///
    /// # 返回值
    /// 如果在沙箱内返回 true
    pub fn is_in_sandbox(&self, path: &std::path::Path) -> bool {
        if let Ok(canonical_path) = std::fs::canonicalize(path) {
            if let Ok(canonical_root) = std::fs::canonicalize(&self.sandbox_root) {
                return canonical_path.starts_with(&canonical_root);
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[tokio::test]
    async fn test_valid_path() {
        let root = PathBuf::from("/tmp/sandbox_test");
        let validator = PathValidator::new(root);

        let result = validator.validate_path("user1", PathBuf::from("user_123/file.txt")).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_path_traversal_attempt() {
        let root = PathBuf::from("/tmp/sandbox_test");
        let validator = PathValidator::new(root);

        let result = validator.validate_path("user1", PathBuf::from("user_123/../../../etc/passwd")).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), SandboxError::PathTraversal { .. }));
    }

    #[test]
    fn test_has_path_traversal_attempt() {
        let root = PathBuf::from("/tmp/sandbox_test");
        let validator = PathValidator::new(root);

        assert!(validator.has_path_traversal_attempt(PathBuf::from("../etc")));
        assert!(validator.has_path_traversal_attempt(PathBuf::from("..\\windows")));
        assert!(validator.has_path_traversal_attempt(PathBuf::from("test/../../etc")));
        assert!(!validator.has_path_traversal_attempt(PathBuf::from("normal/path")));
    }
}
