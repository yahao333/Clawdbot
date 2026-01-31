//! 会话存储模块
//!
//! 提供会话数据的存储和管理接口，支持内存存储和数据库持久化。

use std::sync::Arc;
use std::time::{Duration, SystemTime};
use dashmap::DashMap;
use async_trait::async_trait;
use tracing::{debug, info, warn};
use crate::core::session::{Session, SessionQuery, SessionStats, SessionStatus};

/// 会话存储 trait
///
/// 定义会话存储的基本操作
#[async_trait]
pub trait SessionStore: Send + Sync + std::fmt::Debug {
    /// 创建或获取会话
    async fn get_or_create(
        &self,
        channel: &str,
        target_id: &str,
        sender_id: Option<&str>,
        agent_id: &str,
    ) -> Session;

    /// 获取会话
    async fn get(&self, session_id: &str) -> Option<Session>;

    /// 通过会话键获取会话
    async fn get_by_key(&self, channel: &str, target_id: &str, sender_id: Option<&str>) -> Option<Session>;

    /// 更新会话
    async fn update(&self, session: &Session) -> bool;

    /// 删除会话
    async fn delete(&self, session_id: &str) -> bool;

    /// 查询会话
    async fn query(&self, query: &SessionQuery) -> Vec<Session>;

    /// 获取会话统计
    async fn stats(&self) -> SessionStats;

    /// 清理过期会话
    async fn cleanup_expired(&self, expire_seconds: u64) -> usize;
}

/// 内存会话存储
///
/// 使用 DashMap 实现线程安全的内存存储
///
/// # 特点
/// * 线程安全 - 使用 DashMap 支持并发访问
/// * 快速访问 - O(1) 时间复杂度的查找和更新
/// * 自动过期 - 支持定期清理过期会话
///
/// # 字段说明
/// * `sessions` - 会话存储（ID -> Session）
/// * `session_keys` - 会话键映射（Key -> Session ID）
/// * `config` - 存储配置
#[derive(Debug, Clone)]
pub struct InMemorySessionStore {
    /// 会话存储（ID -> Session）
    sessions: Arc<DashMap<String, Session>>,
    /// 会话键索引（Key -> Session ID）
    session_keys: Arc<DashMap<String, String>>,
    /// 配置
    config: SessionStoreConfig,
}

impl InMemorySessionStore {
    /// 创建新的内存会话存储
    ///
    /// # 参数说明
    /// * `config` - 存储配置
    ///
    /// # 返回值
    /// 创建的内存会话存储实例
    pub fn new(config: Option<SessionStoreConfig>) -> Self {
        let config = config.unwrap_or_default();

        let store = Self {
            sessions: Arc::new(DashMap::new()),
            session_keys: Arc::new(DashMap::new()),
            config,
        };

        info!(
            capacity = store.config.capacity,
            "内存会话存储创建成功"
        );

        store
    }

    /// 创建默认配置的内存会话存储
    pub fn default() -> Self {
        Self::new(None)
    }

    /// 生成会话键
    fn generate_key(channel: &str, target_id: &str, sender_id: Option<&str>) -> String {
        match sender_id {
            Some(sender) => format!("{}:{}:{}", channel, target_id, sender),
            None => format!("{}:{}", channel, target_id),
        }
    }

    /// 清理过期会话的内部方法
    fn cleanup_expired_sync(&self, expire_seconds: u64) -> usize {
        let mut removed_count = 0;
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or(Duration::from_secs(0))
            .as_millis() as i64;

        let expire_ms = expire_seconds as i64 * 1000;

        // 遍历并清理过期会话
        self.sessions.retain(|id, session| {
            let is_expired = (now - session.updated_at) > expire_ms;
            if is_expired {
                // 清理键索引
                let key = session.generate_session_key();
                self.session_keys.remove(&key);
                removed_count += 1;
                tracing::debug!(session_id = %id, "会话已过期，清理");
                false // 移除
            } else {
                true // 保留
            }
        });

        removed_count
    }
}

#[async_trait]
impl SessionStore for InMemorySessionStore {
    async fn get_or_create(
        &self,
        channel: &str,
        target_id: &str,
        sender_id: Option<&str>,
        agent_id: &str,
    ) -> Session {
        let key = Self::generate_key(channel, target_id, sender_id);

        // 尝试查找现有会话
        if let Some(session_id) = self.session_keys.get(&key) {
            if let Some(session) = self.sessions.get(session_id.value()) {
                if !session.is_expired(self.config.expire_seconds) {
                    debug!(
                        session_id = %session.id,
                        key = %key,
                        "找到现有会话"
                    );
                    return session.clone();
                }
                // 会话已过期，删除
                self.sessions.remove(session_id.value());
                self.session_keys.remove(&key);
                let expired_id = session_id.value().clone();
                tracing::debug!(session_id = %expired_id, "会话已过期，删除");
            }
        }

        // 创建新会话
        debug!(key = %key, "创建新会话");
        let session = Session::new(channel, target_id, sender_id, agent_id);

        // 存储会话
        self.sessions.insert(session.id.clone(), session.clone());
        self.session_keys.insert(key, session.id.clone());

        session
    }

    async fn get(&self, session_id: &str) -> Option<Session> {
        self.sessions.get(session_id).map(|s| s.clone())
    }

    async fn get_by_key(&self, channel: &str, target_id: &str, sender_id: Option<&str>) -> Option<Session> {
        let key = Self::generate_key(channel, target_id, sender_id);
        self.session_keys.get(&key)
            .and_then(|session_id| self.sessions.get(session_id.value()).map(|s| s.clone()))
    }

    async fn update(&self, session: &Session) -> bool {
        let key = session.generate_session_key();

        // 更新会话
        let result = self.sessions.insert(session.id.clone(), session.clone());

        // 确保键索引存在
        if result.is_some() {
            self.session_keys.insert(key, session.id.clone());
            debug!(session_id = %session.id, "会话更新成功");
            true
        } else {
            warn!(session_id = %session.id, "会话更新失败，不存在");
            false
        }
    }

    async fn delete(&self, session_id: &str) -> bool {
        if let Some((_, session)) = self.sessions.remove(session_id) {
            let key = session.generate_session_key();
            self.session_keys.remove(&key);
            debug!(session_id = %session_id, "会话删除成功");
            true
        } else {
            debug!(session_id = %session_id, "会话删除失败，不存在");
            false
        }
    }

    async fn query(&self, query: &SessionQuery) -> Vec<Session> {
        self.sessions
            .iter()
            .filter_map(|entry| {
                let session = entry.value();
                // 过滤条件
                if let Some(ref channel) = query.channel {
                    if session.channel != *channel {
                        return None;
                    }
                }
                if let Some(ref target_id) = query.target_id {
                    if session.target_id != *target_id {
                        return None;
                    }
                }
                if let Some(ref sender_id) = query.sender_id {
                    if session.sender_id.as_deref() != Some(sender_id) {
                        return None;
                    }
                }
                if let Some(ref agent_id) = query.agent_id {
                    if session.agent_id != *agent_id {
                        return None;
                    }
                }
                if let Some(ref status) = query.status {
                    if session.status != *status {
                        return None;
                    }
                }
                if !query.include_expired && session.is_expired(self.config.expire_seconds) {
                    return None;
                }
                Some(session.clone())
            })
            .take(query.limit)
            .collect()
    }

    async fn stats(&self) -> SessionStats {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or(Duration::from_secs(0))
            .as_millis() as i64;
        let today_start = now - (now % 86400000); // 今天开始的时间戳
        let expire_ms = self.config.expire_seconds as i64 * 1000;

        let total = self.sessions.len();
        let mut active = 0;
        let mut today_new = 0;
        let mut total_messages = 0;

        for entry in self.sessions.iter() {
            let session = entry.value();
            // 活跃会话
            if (now - session.updated_at) <= expire_ms {
                active += 1;
            }

            // 今日新增
            if session.created_at >= today_start {
                today_new += 1;
            }

            total_messages += session.messages.len();
        }

        let avg_messages = if total > 0 {
            total_messages as f64 / total as f64
        } else {
            0.0
        };

        SessionStats {
            total_sessions: total as u64,
            active_sessions: active as u64,
            today_new_sessions: today_new as u64,
            avg_messages_per_session: avg_messages,
        }
    }

    async fn cleanup_expired(&self, expire_seconds: u64) -> usize {
        self.cleanup_expired_sync(expire_seconds)
    }
}

/// 会话存储配置
#[derive(Debug, Clone)]
pub struct SessionStoreConfig {
    /// 最大存储容量
    pub capacity: usize,
    /// 会话过期时间（秒）
    pub expire_seconds: u64,
    /// 自动清理间隔（秒）
    pub cleanup_interval_secs: u64,
}

impl Default for SessionStoreConfig {
    fn default() -> Self {
        Self {
            capacity: 10000,
            expire_seconds: 3600,
            cleanup_interval_secs: 300,
        }
    }
}
