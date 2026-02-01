//! Sandbox 服务模块
//!
//! 提供沙箱环境的核心服务，管理用户沙箱目录和文件操作
//!
//! # 功能特性
//! - 用户沙箱目录管理
//! - 文件读写操作（带沙箱验证）
//! - 目录操作（列出、创建、删除）
//! - 磁盘配额管理
//!
//! # 使用示例
//! ```rust
//! use clawdbot::sandbox::SandboxService;
//! use std::sync::Arc;
//!
//! let service = SandboxService::new("./data/sandbox").await?;
//!
//! // 读取文件
//! let content = service.read_file("user123", "test.txt").await?;
//!
//! // 写入文件
//! service.write_file("user123", "test.txt", b"hello").await?;
//!
//! // 列出目录
//! let entries = service.list_dir("user123", "").await?;
//! ```

use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

use crate::sandbox::errors::{SandboxError, SandboxResult};
use crate::sandbox::path::{PathValidator, PathValidationResult};
use crate::sandbox::types::{SandboxConfig, SandboxEntry, UserSandbox};

/// 用户沙箱缓存
///
/// 使用 Mutex 保护的用户沙箱信息缓存
#[derive(Clone)]
struct UserSandboxCache {
    /// 用户沙箱映射
    sandboxes: Arc<Mutex<std::collections::HashMap<String, UserSandbox>>>,
}

impl UserSandboxCache {
    fn new() -> Self {
        Self {
            sandboxes: Arc::new(Mutex::new(std::collections::HashMap::new())),
        }
    }

    async fn get(&self, user_id: &str) -> Option<UserSandbox> {
        let sandboxes = self.sandboxes.lock().await;
        sandboxes.get(user_id).cloned()
    }

    async fn insert(&self, user_id: String, sandbox: UserSandbox) {
        let mut sandboxes = self.sandboxes.lock().await;
        sandboxes.insert(user_id, sandbox);
    }

    async fn update_size(&self, user_id: &str, delta: i64) {
        let mut sandboxes = self.sandboxes.lock().await;
        if let Some(sandbox) = sandboxes.get_mut(user_id) {
            if delta >= 0 {
                sandbox.current_size = sandbox.current_size.saturating_add(delta as u64);
            } else {
                sandbox.current_size = sandbox.current_size.saturating_sub((-delta) as u64);
            }
        }
    }

    async fn remove(&self, user_id: &str) {
        let mut sandboxes = self.sandboxes.lock().await;
        sandboxes.remove(user_id);
    }
}

/// 沙箱服务
///
/// 管理所有用户沙箱目录和文件操作的核心服务
///
/// # 字段说明
/// * `config` - 沙箱配置
/// * `path_validator` - 路径验证器
/// * `user_cache` - 用户沙箱缓存
#[derive(Clone)]
pub struct SandboxService {
    /// 沙箱配置
    config: Arc<SandboxConfig>,

    /// 路径验证器
    path_validator: Arc<PathValidator>,

    /// 用户沙箱缓存
    user_cache: UserSandboxCache,
}

impl SandboxService {
    /// 创建沙箱服务
    ///
    /// # 参数说明
    /// * `base_path` - 沙箱根目录路径
    ///
    /// # 返回值
    /// 创建的沙箱服务实例
    ///
    /// # 错误
    /// 如果创建沙箱根目录失败，返回错误
    pub async fn new(base_path: PathBuf) -> SandboxResult<Self> {
        Self::with_config(SandboxConfig {
            base_path,
            ..Default::default()
        }).await
    }

    /// 使用配置创建沙箱服务
    ///
    /// # 参数说明
    /// * `config` - 沙箱配置
    ///
    /// # 返回值
    /// 创建的沙箱服务实例
    pub async fn with_config(config: SandboxConfig) -> SandboxResult<Self> {
        info!(
            enabled = config.enabled,
            base_path = %config.base_path.display(),
            "初始化沙箱服务"
        );

        // 创建路径验证器
        let path_validator = Arc::new(PathValidator::with_config(
            config.base_path.clone(),
            &config,
        ));

        // 确保沙箱根目录存在
        if let Err(e) = tokio::fs::create_dir_all(&config.base_path).await {
            warn!(
                path = %config.base_path.display(),
                error = %e,
                "创建沙箱根目录失败"
            );
            return Err(SandboxError::UserDirCreationFailed {
                path: config.base_path,
                source: e,
            });
        }

        info!(path = %config.base_path.display(), "沙箱服务初始化成功");

        Ok(Self {
            config: Arc::new(config),
            path_validator,
            user_cache: UserSandboxCache::new(),
        })
    }

    /// 获取沙箱配置
    ///
    /// # 返回值
    /// 沙箱配置的引用
    pub fn config(&self) -> &SandboxConfig {
        &self.config
    }

    /// 检查沙箱是否启用
    ///
    /// # 返回值
    /// 如果启用返回 true
    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    /// 获取用户的沙箱目录
    ///
    /// 如果用户沙箱不存在，则创建它
    async fn get_or_create_user_sandbox(&self, user_id: &str) -> SandboxResult<UserSandbox> {
        // 先检查缓存
        if let Some(sandbox) = self.user_cache.get(user_id).await {
            return Ok(sandbox);
        }

        // 创建用户沙箱根目录
        let user_dir_name = self.config.user_dir_format.replace("{user_id}", user_id);
        let user_root = self.config.base_path.join(&user_dir_name);

        debug!(
            user_id = user_id,
            path = %user_root.display(),
            "创建用户沙箱目录"
        );

        // 创建必要的子目录
        let subdirs = ["files", "uploads", "downloads"];
        for subdir in &subdirs {
            let subdir_path = user_root.join(subdir);
            if let Err(e) = tokio::fs::create_dir_all(&subdir_path).await {
                warn!(
                    user_id = user_id,
                    subdir = subdir,
                    error = %e,
                    "创建子目录失败"
                );
                return Err(SandboxError::UserDirCreationFailed {
                    path: subdir_path,
                    source: e,
                });
            }
        }

        // 创建用户沙箱信息
        let max_size_bytes = self.config.max_size_mb * 1024 * 1024;
        let sandbox = UserSandbox::new(user_id, user_root, max_size_bytes);

        // 缓存用户沙箱
        self.user_cache.insert(user_id.to_string(), sandbox.clone()).await;

        info!(
            user_id = user_id,
            path = %sandbox.root_path.display(),
            "用户沙箱创建成功"
        );

        Ok(sandbox)
    }

    /// 验证并获取安全路径
    async fn validate_path(
        &self,
        user_id: &str,
        path: &str,
    ) -> SandboxResult<PathValidationResult> {
        // 检查沙箱是否启用
        if !self.config.enabled {
            return Err(SandboxError::Disabled);
        }

        // 验证路径
        let path_buf = PathBuf::from(path);
        self.path_validator.validate_path(user_id, &path_buf).await
    }

    // ==================== 文件操作 API ====================

    /// 读取文件内容
    pub async fn read_file(
        &self,
        user_id: &str,
        path: &str,
    ) -> SandboxResult<Vec<u8>> {
        let validated = self.validate_path(user_id, path).await?;

        debug!(
            user_id = user_id,
            path = %validated.safe_path.display(),
            "读取文件"
        );

        // 打开并读取文件
        let file = tokio::fs::File::open(&validated.safe_path).await
            .map_err(|e| SandboxError::IoError { source: e })?;

        let mut buffer = Vec::new();
        let mut reader = tokio::io::BufReader::new(file);
        tokio::io::AsyncReadExt::read_to_end(&mut reader, &mut buffer).await
            .map_err(|e| SandboxError::IoError { source: e })?;

        Ok(buffer)
    }

    /// 写入文件内容
    ///
    /// 如果文件不存在则创建，存在则覆盖
    pub async fn write_file(
        &self,
        user_id: &str,
        path: &str,
        content: &[u8],
    ) -> SandboxResult<usize> {
        // 检查磁盘配额
        let sandbox = self.get_or_create_user_sandbox(user_id).await?;
        if sandbox.is_quota_exceeded(content.len() as u64) {
            return Err(SandboxError::QuotaExceeded {
                current_size: sandbox.current_size,
                max_size: sandbox.max_size,
            });
        }

        let validated = self.validate_path(user_id, path).await?;

        debug!(
            user_id = user_id,
            path = %validated.safe_path.display(),
            size = content.len(),
            "写入文件"
        );

        // 确保父目录存在
        if let Some(parent) = validated.safe_path.parent() {
            tokio::fs::create_dir_all(parent).await
                .map_err(|e| SandboxError::IoError { source: e })?;
        }

        // 写入文件
        let mut file = tokio::fs::File::create(&validated.safe_path).await
            .map_err(|e| SandboxError::IoError { source: e })?;

        tokio::io::AsyncWriteExt::write_all(&mut file, content).await
            .map_err(|e| SandboxError::IoError { source: e })?;

        // 更新磁盘使用量
        self.user_cache.update_size(user_id, content.len() as i64).await;

        Ok(content.len())
    }

    /// 追加写入文件
    pub async fn append_file(
        &self,
        user_id: &str,
        path: &str,
        content: &[u8],
    ) -> SandboxResult<usize> {
        let validated = self.validate_path(user_id, path).await?;

        // 检查配额
        let sandbox = self.get_or_create_user_sandbox(user_id).await?;
        if sandbox.is_quota_exceeded(content.len() as u64) {
            return Err(SandboxError::QuotaExceeded {
                current_size: sandbox.current_size,
                max_size: sandbox.max_size,
            });
        }

        // 以追加模式打开文件
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&validated.safe_path)
            .await
            .map_err(|e| SandboxError::IoError { source: e })?;

        tokio::io::AsyncWriteExt::write_all(&mut file, content).await
            .map_err(|e| SandboxError::IoError { source: e })?;

        // 更新配额
        self.user_cache.update_size(user_id, content.len() as i64).await;

        Ok(content.len())
    }

    // ==================== 目录操作 API ====================

    /// 列出目录内容
    pub async fn list_dir(
        &self,
        user_id: &str,
        path: &str,
    ) -> SandboxResult<Vec<SandboxEntry>> {
        let validated = self.validate_path(user_id, path).await?;

        debug!(
            user_id = user_id,
            path = %validated.safe_path.display(),
            "列出目录"
        );

        let mut entries = Vec::new();
        let mut read_dir = tokio::fs::read_dir(&validated.safe_path).await
            .map_err(|e| SandboxError::IoError { source: e })?;

        while let Some(entry) = read_dir.next_entry().await
            .map_err(|e| SandboxError::IoError { source: e })? {
            let entry = SandboxEntry::from_dir_entry(&entry).await
                .map_err(|e| SandboxError::IoError { source: e })?;
            entries.push(entry);
        }

        // 按名称排序
        entries.sort_by(|a, b| a.name.cmp(&b.name));

        Ok(entries)
    }

    /// 创建目录
    pub async fn create_dir(
        &self,
        user_id: &str,
        path: &str,
        recursive: bool,
    ) -> SandboxResult<PathBuf> {
        let validated = self.validate_path(user_id, path).await?;

        debug!(
            user_id = user_id,
            path = %validated.safe_path.display(),
            recursive = recursive,
            "创建目录"
        );

        if recursive {
            tokio::fs::create_dir_all(&validated.safe_path).await
                .map_err(|e| SandboxError::IoError { source: e })?;
        } else {
            tokio::fs::create_dir(&validated.safe_path).await
                .map_err(|e| SandboxError::IoError { source: e })?;
        }

        Ok(validated.safe_path)
    }

    // ==================== 删除操作 API ====================

    /// 删除文件
    pub async fn delete_file(&self, user_id: &str, path: &str) -> SandboxResult<()> {
        let validated = self.validate_path(user_id, path).await?;

        debug!(
            user_id = user_id,
            path = %validated.safe_path.display(),
            "删除文件"
        );

        // 检查是否为文件
        let metadata = tokio::fs::metadata(&validated.safe_path).await
            .map_err(|e| SandboxError::IoError { source: e })?;

        if metadata.is_dir() {
            return Err(SandboxError::PermissionDenied {
                path: validated.safe_path,
            });
        }

        // 获取文件大小用于更新配额
        let file_size = metadata.len();

        // 删除文件
        tokio::fs::remove_file(&validated.safe_path).await
            .map_err(|e| SandboxError::IoError { source: e })?;

        // 更新磁盘使用量
        self.user_cache.update_size(user_id, -(file_size as i64)).await;

        Ok(())
    }

    /// 删除目录
    pub async fn delete_dir(
        &self,
        user_id: &str,
        path: &str,
        recursive: bool,
    ) -> SandboxResult<()> {
        let validated = self.validate_path(user_id, path).await?;

        debug!(
            user_id = user_id,
            path = %validated.safe_path.display(),
            recursive = recursive,
            "删除目录"
        );

        if recursive {
            tokio::fs::remove_dir_all(&validated.safe_path).await
                .map_err(|e| SandboxError::IoError { source: e })?;
        } else {
            tokio::fs::remove_dir(&validated.safe_path).await
                .map_err(|e| SandboxError::IoError { source: e })?;
        }

        Ok(())
    }

    // ==================== 工具 API ====================

    /// 检查文件/目录是否存在
    pub async fn exists(&self, user_id: &str, path: &str) -> bool {
        self.validate_path(user_id, path).await.is_ok()
    }

    /// 获取文件/目录元数据
    pub async fn metadata(
        &self,
        user_id: &str,
        path: &str,
    ) -> SandboxResult<SandboxEntry> {
        let validated = self.validate_path(user_id, path).await?;

        let metadata = tokio::fs::metadata(&validated.safe_path).await
            .map_err(|e| SandboxError::IoError { source: e })?;

        let name = validated.safe_path.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        let modified = metadata.modified()
            .map_err(|e| SandboxError::IoError { source: e })?;

        Ok(SandboxEntry {
            name,
            path: validated.safe_path,
            is_dir: metadata.is_dir(),
            size: if metadata.is_file() { metadata.len() } else { 0 },
            modified,
        })
    }

    /// 获取用户磁盘使用情况
    pub async fn get_usage(&self, user_id: &str) -> (u64, u64) {
        if let Some(sandbox) = self.user_cache.get(user_id).await {
            (sandbox.current_size, sandbox.max_size)
        } else if let Ok(sandbox) = self.get_or_create_user_sandbox(user_id).await {
            (sandbox.current_size, sandbox.max_size)
        } else {
            (0, self.config.max_size_mb * 1024 * 1024)
        }
    }

    /// 清理用户沙箱
    pub async fn cleanup(&self, user_id: &str) -> SandboxResult<u64> {
        let sandbox = self.get_or_create_user_sandbox(user_id).await?;

        debug!(
            user_id = user_id,
            path = %sandbox.root_path.display(),
            "清理用户沙箱"
        );

        let count = if sandbox.root_path.exists() {
            let mut count = 0;
            let mut dir = tokio::fs::read_dir(&sandbox.root_path).await
                .map_err(|e| SandboxError::IoError { source: e })?;

            while let Some(entry) = dir.next_entry().await
                .map_err(|e| SandboxError::IoError { source: e })? {
                let path = entry.path();
                if path.is_dir() {
                    tokio::fs::remove_dir_all(&path).await
                        .map_err(|e| SandboxError::IoError { source: e })?;
                } else {
                    tokio::fs::remove_file(&path).await
                        .map_err(|e| SandboxError::IoError { source: e })?;
                }
                count += 1;
            }

            count
        } else {
            0
        };

        // 清除缓存
        self.user_cache.remove(user_id).await;

        // 清除目录
        let _ = tokio::fs::remove_dir_all(&sandbox.root_path).await;

        info!(user_id = user_id, deleted_count = count, "用户沙箱清理完成");

        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    async fn create_test_service() -> (SandboxService, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let service = SandboxService::new(temp_dir.path().to_path_buf()).await.unwrap();
        (service, temp_dir)
    }

    #[tokio::test]
    async fn test_read_write_file() {
        let (service, _temp) = create_test_service().await;

        let result = service.write_file("user1", "test.txt", b"Hello, World!").await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 13);

        let content = service.read_file("user1", "test.txt").await;
        assert!(content.is_ok());
        assert_eq!(content.unwrap(), b"Hello, World!");
    }

    #[tokio::test]
    async fn test_list_dir() {
        let (service, _temp) = create_test_service().await;

        service.write_file("user1", "file1.txt", b"content1").await.unwrap();
        service.write_file("user1", "file2.txt", b"content2").await.unwrap();
        service.create_dir("user1", "subdir", true).await.unwrap();

        let entries = service.list_dir("user1", "").await.unwrap();
        assert_eq!(entries.len(), 3);

        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"file1.txt"));
        assert!(names.contains(&"file2.txt"));
        assert!(names.contains(&"subdir"));
    }

    #[tokio::test]
    async fn test_path_traversal_protection() {
        let (service, _temp) = create_test_service().await;

        let result = service.read_file("user1", "../outside.txt").await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), SandboxError::PathTraversal { .. }));
    }

    #[tokio::test]
    async fn test_get_usage() {
        let (service, _temp) = create_test_service().await;

        service.write_file("user1", "test.txt", b"Hello").await.unwrap();

        let (used, max) = service.get_usage("user1").await;
        assert!(used > 0);
        assert_eq!(max, 100 * 1024 * 1024);
    }
}
