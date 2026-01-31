//! SQLite 会话存储实现
//!
//! 使用 SQLite 数据库持久化存储会话数据。

use std::sync::Arc;
use std::time::{Duration, SystemTime};
use std::collections::HashMap;
use async_trait::async_trait;
use tracing::{debug, error, info, warn};
use crate::core::session::{Session, SessionQuery, SessionStats, SessionStatus, SessionMessage, SessionConfig};
use crate::core::session::store::SessionStore;
use crate::infra::db::{Database, SessionRecord, MessageRecord};

/// SQLite 会话存储
#[derive(Debug, Clone)]
pub struct SqliteSessionStore {
    /// 数据库实例
    db: Database,
    /// 会话配置
    config: Arc<SessionConfig>,
}

impl SqliteSessionStore {
    /// 创建新的 SQLite 会话存储
    pub async fn new(db_path: &str, config: SessionConfig) -> Result<Self, crate::infra::error::Error> {
        let db = Database::new(db_path).await?;
        Ok(Self {
            db,
            config: Arc::new(config),
        })
    }

    /// 将 SessionRecord 转换为 Session
    async fn record_to_session(&self, record: SessionRecord) -> Session {
        let messages = match self.db.get_session_messages(&record.id).await {
            Ok(msgs) => msgs.into_iter().map(Self::record_to_message).collect(),
            Err(e) => {
                error!(session_id = %record.id, error = %e, "获取会话消息失败");
                Vec::new()
            }
        };

        let metadata: HashMap<String, String> = serde_json::from_str(&record.metadata).unwrap_or_default();
        
        let status = match record.status.as_str() {
            "Active" => SessionStatus::Active,
            "Paused" => SessionStatus::Paused,
            "Ended" => SessionStatus::Ended,
            "Expired" => SessionStatus::Expired,
            _ => SessionStatus::Active,
        };

        Session {
            id: record.id,
            channel: record.channel,
            target_id: record.target_id,
            sender_id: if record.sender_id.is_empty() { None } else { Some(record.sender_id) },
            agent_id: record.agent_id,
            status,
            created_at: record.created_at,
            updated_at: record.updated_at,
            messages,
            metadata,
        }
    }

    /// 将 MessageRecord 转换为 SessionMessage
    fn record_to_message(record: MessageRecord) -> SessionMessage {
        SessionMessage {
            id: record.id,
            role: record.role,
            content: record.content,
            created_at: record.created_at,
        }
    }
}

#[async_trait]
impl SessionStore for SqliteSessionStore {
    async fn get_or_create(
        &self,
        channel: &str,
        target_id: &str,
        sender_id: Option<&str>,
        agent_id: &str,
    ) -> Session {
        // 尝试从数据库获取
        match self.db.get_session_by_key(channel, target_id, sender_id).await {
            Ok(Some(record)) => {
                let session = self.record_to_session(record).await;
                // 检查是否过期
                if session.is_expired(self.config.expire_seconds) {
                    debug!(session_id = %session.id, "会话已过期 (DB)");
                    // 可以在这里更新状态为 Expired，或者直接创建新的
                    // 简单起见，如果过期就创建新的 (类似于 InMemory)
                    // 但通常数据库存储会保留记录。这里为了保持行为一致，我们创建新的，旧的保留在数据库中作为历史?
                    // 或者更新旧的状态为 Expired 并创建新的。
                    // 让我们创建一个新的会话，不覆盖旧的 (旧的 ID 不同)
                } else {
                    debug!(session_id = %session.id, "找到现有会话 (DB)");
                    return session;
                }
            }
            Ok(None) => {},
            Err(e) => {
                error!(error = %e, "查找会话失败");
            }
        }

        // 创建新会话
        debug!(channel = %channel, target_id = %target_id, "创建新会话 (DB)");
        let session = Session::new(channel, target_id, sender_id, agent_id);
        
        // 保存到数据库
        let record = SessionRecord {
            id: session.id.clone(),
            channel: session.channel.clone(),
            target_id: session.target_id.clone(),
            sender_id: session.sender_id.clone().unwrap_or_default(),
            agent_id: session.agent_id.clone(),
            status: "Active".to_string(),
            created_at: session.created_at,
            updated_at: session.updated_at,
            metadata: serde_json::to_string(&session.metadata).unwrap_or_else(|_| "{}".to_string()),
        };

        if let Err(e) = self.db.create_session(&record).await {
            error!(error = %e, "保存新会话失败");
        }

        session
    }

    async fn get(&self, session_id: &str) -> Option<Session> {
        match self.db.get_session(session_id).await {
            Ok(Some(record)) => Some(self.record_to_session(record).await),
            Ok(None) => None,
            Err(e) => {
                error!(session_id = %session_id, error = %e, "获取会话失败");
                None
            }
        }
    }

    async fn get_by_key(&self, channel: &str, target_id: &str, sender_id: Option<&str>) -> Option<Session> {
        match self.db.get_session_by_key(channel, target_id, sender_id).await {
            Ok(Some(record)) => Some(self.record_to_session(record).await),
            Ok(None) => None,
            Err(e) => {
                error!(error = %e, "通过Key获取会话失败");
                None
            }
        }
    }

    async fn update(&self, session: &Session) -> bool {
        // 更新会话信息
        let record = SessionRecord {
            id: session.id.clone(),
            channel: session.channel.clone(),
            target_id: session.target_id.clone(),
            sender_id: session.sender_id.clone().unwrap_or_default(),
            agent_id: session.agent_id.clone(),
            status: match session.status {
                SessionStatus::Active => "Active",
                SessionStatus::Paused => "Paused",
                SessionStatus::Ended => "Ended",
                SessionStatus::Expired => "Expired",
            }.to_string(),
            created_at: session.created_at,
            updated_at: session.updated_at,
            metadata: serde_json::to_string(&session.metadata).unwrap_or_else(|_| "{}".to_string()),
        };

        match self.db.update_session(&record).await {
            Ok(_) => {
                // 同步消息
                // 为了简化实现且保证一致性，我们只添加新消息
                // 获取数据库中现有的消息ID列表
                 if let Ok(db_messages) = self.db.get_session_messages(&session.id).await {
                    let db_msg_ids: std::collections::HashSet<String> = db_messages.into_iter().map(|m| m.id).collect();
                    
                    for msg in &session.messages {
                        if !db_msg_ids.contains(&msg.id) {
                            let msg_record = MessageRecord {
                                id: msg.id.clone(),
                                session_id: session.id.clone(),
                                role: msg.role.clone(),
                                content: msg.content.clone(),
                                created_at: msg.created_at,
                            };
                            if let Err(e) = self.db.add_message(&msg_record).await {
                                error!(msg_id = %msg.id, error = %e, "添加消息失败");
                            }
                        }
                    }
                }
                
                true
            },
            Err(e) => {
                error!(session_id = %session.id, error = %e, "更新会话失败");
                false
            }
        }
    }

    async fn delete(&self, session_id: &str) -> bool {
        match self.db.delete_session(session_id).await {
            Ok(success) => success,
            Err(e) => {
                error!(session_id = %session_id, error = %e, "删除会话失败");
                false
            }
        }
    }

    async fn query(&self, query: &SessionQuery) -> Vec<Session> {
        let status_str = query.status.as_ref().map(|s| match s {
            SessionStatus::Active => "Active",
            SessionStatus::Paused => "Paused",
            SessionStatus::Ended => "Ended",
            SessionStatus::Expired => "Expired",
        });

        match self.db.query_sessions(
            query.channel.as_deref(),
            query.target_id.as_deref(),
            query.sender_id.as_deref(),
            query.agent_id.as_deref(),
            status_str,
            query.limit
        ).await {
            Ok(records) => {
                let mut sessions = Vec::new();
                for record in records {
                    sessions.push(self.record_to_session(record).await);
                }
                
                // 内存中再次过滤 (如 include_expired)
                if !query.include_expired {
                     sessions.retain(|s| !s.is_expired(self.config.expire_seconds));
                }
                
                sessions
            }
            Err(e) => {
                error!(error = %e, "查询会话失败");
                Vec::new()
            }
        }
    }

    async fn stats(&self) -> SessionStats {
        match self.db.get_session_stats().await {
            Ok((total, active, today_new)) => {
                // 平均消息数需要额外查询，这里简化处理或者再加一个查询
                // 暂时设为 0.0 或者通过其他方式获取
                SessionStats {
                    total_sessions: total as u64,
                    active_sessions: active as u64,
                    today_new_sessions: today_new as u64,
                    avg_messages_per_session: 0.0, // TODO: Implement avg message stats in DB
                }
            },
            Err(e) => {
                error!(error = %e, "获取统计失败");
                SessionStats::default()
            }
        }
    }

    async fn cleanup_expired(&self, expire_seconds: u64) -> usize {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or(Duration::from_secs(0))
            .as_millis() as i64;
        
        let expire_ms = expire_seconds as i64 * 1000;
        let expire_timestamp = now - expire_ms;

        match self.db.cleanup_expired_sessions(expire_timestamp).await {
            Ok(count) => count as usize,
            Err(e) => {
                error!(error = %e, "清理过期会话失败");
                0
            }
        }
    }
}
