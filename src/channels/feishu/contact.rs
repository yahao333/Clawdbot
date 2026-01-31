//! 飞书联系人模块
//!
//! 提供用户信息获取和缓存功能。
//!
//! # 功能
//! 1. 根据用户 ID 获取用户信息（昵称、头像等）
//! 2. 用户信息缓存（减少 API 调用）
//!
//! # 使用示例
//! ```rust
//! let contact_client = ContactClient::new(feishu_client);
//! let user_info = contact_client.get_user("ou_xxx").await?;
//! ```

use crate::channels::feishu::client::FeishuClient;
use crate::infra::error::{Error, Result};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{debug, warn};

/// 飞书用户信息
///
/// 包含用户的基本信息
#[derive(Debug, Clone)]
pub struct FeishuUserInfo {
    /// 用户 ID
    pub user_id: String,
    /// Open ID
    pub open_id: Option<String>,
    /// Union ID
    pub union_id: Option<String>,
    /// 名称
    pub name: String,
    /// 英文名
    pub english_name: Option<String>,
    /// 头像 URL
    pub avatar_url: Option<String>,
    /// 头像缩略图 URL
    pub avatar_thumb: Option<String>,
    /// 性别（0: 未设置, 1: 男, 2: 女）
    pub gender: Option<i32>,
    /// 是否激活
    pub is_activated: Option<bool>,
    /// 用户类型
    pub user_type: Option<i32>,
}

/// 联系人客户端
///
/// 用于获取和管理飞书用户信息
///
/// # 缓存策略
/// - 使用 DashMap 进行线程安全的内存缓存
/// - 缓存有效期：30 分钟
/// - 避免频繁调用 API 导致限流
#[derive(Clone)]
pub struct ContactClient {
    /// 飞书客户端
    feishu_client: FeishuClient,
    /// 用户信息缓存
    user_cache: Arc<DashMap<String, (FeishuUserInfo, i64)>>,
    /// 缓存过期时间（秒）
    cache_ttl: i64,
}

impl ContactClient {
    /// 创建联系人客户端
    pub fn new(feishu_client: FeishuClient, cache_ttl_seconds: i64) -> Self {
        Self {
            feishu_client,
            user_cache: Arc::new(DashMap::new()),
            cache_ttl: cache_ttl_seconds,
        }
    }

    /// 创建默认配置的联系人客户端（30 分钟缓存）
    pub fn new_default(feishu_client: FeishuClient) -> Self {
        Self::new(feishu_client, 1800)
    }

    /// 获取用户信息
    ///
    /// 优先从缓存获取，如果缓存过期或不存在则调用 API
    ///
    /// # 参数说明
    /// * `user_id` - 用户 ID
    ///
    /// # 返回值
    /// 用户信息
    pub async fn get_user(&self, user_id: &str) -> Result<FeishuUserInfo> {
        // 1. 检查缓存
        let now = chrono::Utc::now().timestamp();
        if let Some(cached) = self.user_cache.get(user_id) {
            let (user_info, expires_at) = &*cached;
            if *expires_at > now {
                debug!(user_id = %user_id, "从缓存获取用户信息");
                return Ok(user_info.clone());
            }
        }

        // 2. 缓存未命中或已过期，调用 API
        debug!(user_id = %user_id, "缓存未命中，调用 API 获取用户信息");
        let user_info = self.fetch_user_from_api(user_id).await?;

        // 3. 更新缓存
        let expires_at = now + self.cache_ttl;
        self.user_cache.insert(user_id.to_string(), (user_info.clone(), expires_at));

        Ok(user_info)
    }

    /// 批量获取用户信息
    ///
    /// 减少 API 调用次数，适用于群组消息处理
    ///
    /// # 参数说明
    /// * `user_ids` - 用户 ID 列表
    ///
    /// # 返回值
    /// 用户信息映射（user_id -> user_info）
    pub async fn get_users(&self, user_ids: &[String]) -> Result<Vec<FeishuUserInfo>> {
        let mut result = Vec::new();
        let mut missing_ids = Vec::new();

        let now = chrono::Utc::now().timestamp();

        // 1. 先检查缓存
        for user_id in user_ids {
            if let Some(cached) = self.user_cache.get(user_id) {
                let (user_info, expires_at) = &*cached;
                if *expires_at > now {
                    result.push(user_info.clone());
                } else {
                    missing_ids.push(user_id.clone());
                }
            } else {
                missing_ids.push(user_id.clone());
            }
        }

        // 2. 获取未缓存的用户信息
        if !missing_ids.is_empty() {
            debug!(count = missing_ids.len(), "批量获取用户信息");
            for user_id in missing_ids {
                if let Ok(user_info) = self.fetch_user_from_api(&user_id).await {
                    let expires_at = now + self.cache_ttl;
                    self.user_cache.insert(user_id.clone(), (user_info.clone(), expires_at));
                    result.push(user_info);
                } else {
                    // 如果获取失败，添加一个占位信息
                    result.push(FeishuUserInfo {
                        user_id: user_id.clone(),
                        open_id: None,
                        union_id: None,
                        name: format!("用户({})", &user_id[..8]),
                        english_name: None,
                        avatar_url: None,
                        avatar_thumb: None,
                        gender: None,
                        is_activated: None,
                        user_type: None,
                    });
                }
            }
        }

        Ok(result)
    }

    /// 从 API 获取用户信息
    ///
    /// 调用飞书 Contact API 获取用户详细信息
    async fn fetch_user_from_api(&self, user_id: &str) -> Result<FeishuUserInfo> {
        let path = format!("/contact/v1/users/{}", user_id);

        #[derive(Deserialize)]
        struct ApiData {
            user: Option<UserResponse>,
        }

        #[derive(Deserialize)]
        struct UserResponse {
            #[serde(rename = "user_id")]
            user_id: String,
            #[serde(rename = "open_id")]
            open_id: Option<String>,
            #[serde(rename = "union_id")]
            union_id: Option<String>,
            name: String,
            #[serde(rename = "english_name")]
            english_name: Option<String>,
            #[serde(rename = "avatar_url")]
            avatar_url: Option<String>,
            #[serde(rename = "avatar_thumb")]
            avatar_thumb: Option<AvatarThumb>,
            gender: Option<i32>,
            #[serde(rename = "is_activated")]
            is_activated: Option<bool>,
            #[serde(rename = "user_type")]
            user_type: Option<i32>,
        }

        #[derive(Deserialize)]
        struct AvatarThumb {
            #[serde(rename = "avatar_url")]
            avatar_url: Option<String>,
        }

        let response: ApiData = self.feishu_client.request("GET", &path, None::<serde_json::Value>).await?;

        let user = response.user
            .ok_or_else(|| Error::Channel("用户不存在".to_string()))?;

        Ok(FeishuUserInfo {
            user_id: user.user_id,
            open_id: user.open_id,
            union_id: user.union_id,
            name: user.name,
            english_name: user.english_name,
            avatar_url: user.avatar_url,
            avatar_thumb: user.avatar_thumb.map(|a| a.avatar_url).flatten(),
            gender: user.gender,
            is_activated: user.is_activated,
            user_type: user.user_type,
        })
    }

    /// 清除缓存
    ///
    /// 强制刷新所有缓存的用户信息
    pub fn clear_cache(&self) {
        self.user_cache.clear();
        debug!("用户信息缓存已清除");
    }

    /// 移除指定用户的缓存
    ///
    /// # 参数说明
    /// * `user_id` - 要移除缓存的用户 ID
    pub fn invalidate(&self, user_id: &str) {
        self.user_cache.remove(user_id);
        debug!(user_id = %user_id, "用户缓存已失效");
    }

    /// 获取缓存统计信息
    pub fn cache_stats(&self) -> (usize, i64) {
        let now = chrono::Utc::now().timestamp();
        let mut valid_count = 0;

        for entry in self.user_cache.iter() {
            let (_, expires_at) = &*entry;
            if *expires_at > now {
                valid_count += 1;
            }
        }

        (valid_count, self.user_cache.len() as i64)
    }
}

/// 飞书部门信息
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FeishuDepartment {
    /// 部门 ID
    #[serde(rename = "department_id")]
    pub department_id: String,
    /// 部门名称
    pub name: String,
    /// 父部门 ID
    #[serde(rename = "parent_department_id")]
    pub parent_id: Option<String>,
    /// 部门排序
    pub order: Option<i64>,
    /// 部门主管
    pub leader_user_id: Option<String>,
}

/// 批量用户信息请求
#[derive(Serialize)]
struct BatchUserRequest {
    /// 用户 ID 列表
    #[serde(rename = "user_ids")]
    user_ids: Vec<String>,
}
