//! 会话管理器
//!
//! 提供会话管理的核心功能，包括会话生命周期管理、历史消息处理和去重。

use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};
use crate::core::message::types::{InboundMessage, MessageContent};
use crate::core::session::{Session, SessionConfig, SessionMessage, SessionQuery, SessionStats, SessionStatus};
use crate::core::session::store::{SessionStore, InMemorySessionStore};
use crate::core::session::sqlite_store::SqliteSessionStore;

/// 会话管理器
///
/// 协调会话存储和消息处理，提供会话生命周期管理
///
/// # 功能
/// * 会话创建和获取
/// * 消息历史管理
/// * 会话去重
/// * 过期清理
///
/// # 字段说明
/// * `store` - 会话存储
/// * `config` - 会话配置
/// * `running` - 运行状态标志
#[derive(Debug, Clone)]
pub struct SessionManager {
    /// 会话存储
    store: Arc<dyn SessionStore>,
    /// 会话配置
    config: Arc<SessionConfig>,
    /// 运行状态
    running: Arc<RwLock<bool>>,
    /// 消息去重缓存 (message_id -> session_id)
    message_dedupe: Arc<dashmap::DashMap<String, (String, i64)>>,
}

impl SessionManager {
    /// 创建新的会话管理器
    ///
    /// # 参数说明
    /// * `config` - 会话配置
    ///
    /// # 返回值
    /// 创建的会话管理器实例
    pub async fn new(config: Option<SessionConfig>) -> Self {
        let config = config.unwrap_or_default();
        
        let store: Arc<dyn SessionStore> = if config.persist_enabled {
            match SqliteSessionStore::new(&config.db_path, config.clone()).await {
                Ok(s) => {
                    info!(path = %config.db_path, "SQLite 会话存储初始化成功");
                    Arc::new(s)
                },
                Err(e) => {
                    error!(error = %e, "SQLite 会话存储初始化失败，回退到内存存储");
                    Arc::new(InMemorySessionStore::new(None))
                }
            }
        } else {
            Arc::new(InMemorySessionStore::new(None))
        };

        let config = Arc::new(config);

        Self {
            store,
            config,
            running: Arc::new(RwLock::new(false)),
            message_dedupe: Arc::new(dashmap::DashMap::new()),
        }
    }

    /// 获取或创建会话
    ///
    /// 根据消息自动获取或创建会话
    ///
    /// # 参数说明
    /// * `message` - 入站消息
    /// * `agent_id` - Agent ID
    ///
    /// # 返回值
    /// 会话实例
    pub async fn get_or_create_session(
        &self,
        message: &InboundMessage,
        agent_id: &str,
    ) -> Session {
        self.store.get_or_create(
            &message.channel,
            &message.target.id,
            Some(&message.sender.id),
            agent_id,
        ).await
    }

    /// 获取会话
    ///
    /// # 参数说明
    /// * `session_id` - 会话ID
    ///
    /// # 返回值
    /// 会话实例或 None
    pub async fn get_session(&self, session_id: &str) -> Option<Session> {
        self.store.get(session_id).await
    }

    /// 更新会话
    ///
    /// # 参数说明
    /// * `session` - 会话实例
    ///
    /// # 返回值
    /// 更新是否成功
    pub async fn update_session(&self, session: &Session) -> bool {
        self.store.update(session).await
    }

    /// 添加消息到会话历史
    ///
    /// # 参数说明
    /// * `session_id` - 会话ID
    /// * `role` - 消息角色（user/assistant/system）
    /// * `content` - 消息内容
    ///
    /// # 返回值
    /// 更新后的会话或 None
    pub async fn add_message_to_session(
        &self,
        session_id: &str,
        role: &str,
        content: &str,
    ) -> Option<Session> {
        let mut session = self.store.get(session_id).await?;

        // 添加消息
        session.add_message(role, content);

        // 截断历史（如果需要）
        if session.messages.len() > self.config.max_history {
            session.truncate_history(self.config.max_history);
        }

        // 更新会话
        self.store.update(&session).await;

        Some(session)
    }

    /// 消息去重检查
    ///
    /// 使用 message_id 检查消息是否已处理
    ///
    /// # 参数说明
    /// * `message_id` - 消息ID
    /// * `session_id` - 会话ID
    ///
    /// # 返回值
    /// * `true` - 消息已处理，跳过
    /// * `false` - 新消息，继续处理
    pub async fn is_message_processed(&self, message_id: &str, session_id: &str) -> bool {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or(Duration::from_secs(0))
            .as_millis() as i64;

        // 清理过期条目（5分钟）
        self.message_dedupe.retain(|_, (_, time)| now - *time < 300000);

        // 检查是否已处理
        if let Some(entry) = self.message_dedupe.get(message_id) {
            let (sess_id, _) = entry.value();
            // 如果在同一个会话中已处理，跳过
            if sess_id.as_str() == session_id {
                debug!(message_id = %message_id, "消息已处理过，跳过");
                return true;
            }
        }

        // 标记为已处理
        self.message_dedupe.insert(message_id.to_string(), (session_id.to_string(), now));
        false
    }

    /// 检查消息是否重复（基于会话+内容）
    ///
    /// # 参数说明
    /// * `session_id` - 会话ID
    /// * `content_hash` - 内容哈希
    ///
    /// # 返回值
    /// 是否重复
    pub async fn is_duplicate_in_session(&self, session_id: &str, content_hash: &str) -> bool {
        if let Some(session) = self.store.get(session_id).await {
            // 检查最近一条消息是否相同
            if let Some(last_msg) = session.messages.last() {
                let last_hash = Self::hash_content(&last_msg.content);
                return last_hash == content_hash;
            }
        }
        false
    }

    /// 计算内容哈希
    fn hash_content(content: &str) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        content.hash(&mut hasher);
        format!("{:x}", hasher.finish())
    }

    /// 获取会话历史
    ///
    /// # 参数说明
    /// * `session_id` - 会话ID
    /// * `limit` - 最大消息数
    ///
    /// # 返回值
    /// 消息历史列表
    pub async fn get_session_history(&self, session_id: &str, limit: Option<usize>) -> Vec<SessionMessage> {
        if let Some(session) = self.store.get(session_id).await {
            let limit = limit.unwrap_or(self.config.max_history);
            session.messages.into_iter().rev().take(limit).rev().collect()
        } else {
            Vec::new()
        }
    }

    /// 查询会话
    ///
    /// # 参数说明
    /// * `query` - 查询条件
    ///
    /// # 返回值
    /// 匹配的会话列表
    pub async fn query_sessions(&self, query: SessionQuery) -> Vec<Session> {
        self.store.query(&query).await
    }

    /// 获取会话统计
    pub async fn get_stats(&self) -> SessionStats {
        self.store.stats().await
    }

    /// 清理过期会话
    pub async fn cleanup_expired(&self) -> usize {
        self.store.cleanup_expired(self.config.expire_seconds).await
    }

    /// 结束会话
    ///
    /// # 参数说明
    /// * `session_id` - 会话ID
    ///
    /// # 返回值
    /// 是否成功
    pub async fn end_session(&self, session_id: &str) -> bool {
        if let Some(mut session) = self.store.get(session_id).await {
            session.status = SessionStatus::Ended;
            self.store.update(&session).await
        } else {
            false
        }
    }

    /// 暂停会话
    pub async fn pause_session(&self, session_id: &str) -> bool {
        if let Some(mut session) = self.store.get(session_id).await {
            session.status = SessionStatus::Paused;
            self.store.update(&session).await
        } else {
            false
        }
    }

    /// 获取总会话数
    pub async fn session_count(&self) -> usize {
        let stats = self.store.stats().await;
        stats.total_sessions as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_session_creation() {
        let config = SessionConfig {
            persist_enabled: false,
            ..Default::default()
        };
        let manager = SessionManager::new(Some(config)).await;

        let session = manager.store.get_or_create(
            "feishu",
            "chat_123",
            Some("user_456"),
            "default",
        ).await;

        assert!(session.id.starts_with("sess_"));
        assert_eq!(session.channel, "feishu");
        assert_eq!(session.target_id, "chat_123");
        assert_eq!(session.sender_id, Some("user_456".to_string()));
        assert_eq!(session.agent_id, "default");
    }

    #[tokio::test]
    async fn test_session_dedupe() {
        let config = SessionConfig {
            persist_enabled: false,
            ..Default::default()
        };
        let manager = SessionManager::new(Some(config)).await;

        let session = manager.store.get_or_create(
            "feishu",
            "chat_123",
            Some("user_456"),
            "default",
        ).await;

        let message_id = "msg_001";

        // 第一次检查，应该返回 false
        assert!(!manager.is_message_processed(message_id, &session.id).await);

        // 第二次检查，应该返回 true
        assert!(manager.is_message_processed(message_id, &session.id).await);
    }

    #[tokio::test]
    async fn test_session_message_history() {
        let config = SessionConfig {
            persist_enabled: false,
            ..Default::default()
        };
        let manager = SessionManager::new(Some(config)).await;

        let session = manager.store.get_or_create(
            "feishu",
            "chat_123",
            Some("user_456"),
            "default",
        ).await;

        // 添加消息
        manager.add_message_to_session(&session.id, "user", "Hello").await;
        manager.add_message_to_session(&session.id, "assistant", "Hi there!").await;

        // 获取历史
        let history = manager.get_session_history(&session.id, None).await;

        assert_eq!(history.len(), 2);
        assert_eq!(history[0].role, "user");
        assert_eq!(history[0].content, "Hello");
        assert_eq!(history[1].role, "assistant");
        assert_eq!(history[1].content, "Hi there!");
    }
}
