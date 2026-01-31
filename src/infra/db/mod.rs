//! 数据库模块
//!
//! 本模块提供了 SQLite 数据库操作接口，使用 sqlx 实现。
//!
//! # 功能
//! 1. 会话存储
//! 2. 消息历史
//! 3. 配置存储
//!
//! # 使用示例
//! ```rust
//! let db = Database::new("clawdbot.db").await?;
//! let sessions = db.get_session("session_key").await?;
//! ```

use sqlx::{sqlite::SqlitePool, Sqlite};
use std::path::PathBuf;
use tracing::{debug, info, warn};

/// 数据库连接池
pub type Pool = SqlitePool;

/// 数据库结构
///
/// 提供数据库操作方法
#[derive(Clone, Debug)]
pub struct Database {
    /// 连接池
    pool: Pool,
}

impl Database {
    /// 创建数据库（简化版）
    ///
    /// # 参数说明
    /// * `path` - 数据库文件路径
    ///
    /// # 返回值
    /// 创建的数据库实例
    pub async fn new(path: &str) -> Result<Self, super::error::Error> {
        Self::new_with_pool(path, 5).await
    }

    /// 创建数据库（完整版）
    ///
    /// # 参数说明
    /// * `path` - 数据库文件路径
    /// * `max_connections` - 最大连接数
    ///
    /// # 返回值
    /// 创建的数据库实例
    pub async fn new_with_pool(
        path: &str,
        max_connections: u32,
    ) -> Result<Self, super::error::Error> {
        let db_path = PathBuf::from(path);
        debug!(path = %db_path.display(), "创建数据库连接");

        // 确保父目录存在
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| super::error::Error::Io(e.to_string()))?;
        }

        // 创建连接 URL（使用绝对路径）
        let absolute_path = std::fs::canonicalize(&db_path)
            .unwrap_or(db_path.clone());
        let url = format!("sqlite:{}", absolute_path.display());

        // 创建连接池
        let pool = SqlitePool::connect_with(
            sqlx::sqlite::SqliteConnectOptions::new()
                .filename(absolute_path)
                .create_if_missing(true)
        )
            .await
            .map_err(|e| super::error::Error::Database(e.to_string()))?;

        // 运行迁移
        Self::run_migrations(&pool).await?;

        info!(path = %db_path.display(), "数据库连接成功");

        Ok(Self { pool })
    }

    /// 运行数据库迁移
    async fn run_migrations(pool: &Pool) -> Result<(), super::error::Error> {
        debug!("运行数据库迁移");

        // 创建会话表
        sqlx::query(r#"
            CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                channel TEXT NOT NULL,
                target_id TEXT NOT NULL,
                sender_id TEXT NOT NULL,
                agent_id TEXT NOT NULL,
                status TEXT DEFAULT 'Active',
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                metadata TEXT DEFAULT '{}'
            )
        "#).execute(pool).await
            .map_err(|e| super::error::Error::Database(e.to_string()))?;

        // 尝试添加新字段（如果表已存在且缺少字段）
        // 忽略错误，因为如果字段已存在会报错
        let _ = sqlx::query("ALTER TABLE sessions ADD COLUMN status TEXT DEFAULT 'Active'").execute(pool).await;
        let _ = sqlx::query("ALTER TABLE sessions ADD COLUMN metadata TEXT DEFAULT '{}'").execute(pool).await;

        // 创建消息历史表
        sqlx::query(r#"
            CREATE TABLE IF NOT EXISTS messages (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                FOREIGN KEY (session_id) REFERENCES sessions(id)
            )
        "#).execute(pool).await
            .map_err(|e| super::error::Error::Database(e.to_string()))?;

        // 创建配置表
        sqlx::query(r#"
            CREATE TABLE IF NOT EXISTS configs (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL,
                updated_at INTEGER NOT NULL
            )
        "#).execute(pool).await
            .map_err(|e| super::error::Error::Database(e.to_string()))?;

        info!("数据库迁移完成");
        Ok(())
    }

    /// 获取连接池
    pub fn pool(&self) -> &Pool {
        &self.pool
    }
}

/// 会话记录
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SessionRecord {
    /// 会话 ID
    pub id: String,
    /// 渠道类型
    pub channel: String,
    /// 目标 ID
    pub target_id: String,
    /// 发送者 ID
    pub sender_id: String,
    /// Agent ID
    pub agent_id: String,
    /// 会话状态
    pub status: String,
    /// 创建时间
    pub created_at: i64,
    /// 更新时间
    pub updated_at: i64,
    /// 元数据 (JSON string)
    pub metadata: String,
}

/// 消息记录
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct MessageRecord {
    /// 消息 ID
    pub id: String,
    /// 会话 ID
    pub session_id: String,
    /// 角色
    pub role: String,
    /// 内容
    pub content: String,
    /// 创建时间
    pub created_at: i64,
}

/// 配置记录
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ConfigRecord {
    /// 配置键
    pub key: String,
    /// 配置值
    pub value: String,
    /// 更新时间
    pub updated_at: i64,
}

impl Database {
    /// 获取会话
    pub async fn get_session(
        &self,
        session_id: &str,
    ) -> Result<Option<SessionRecord>, super::error::Error> {
        sqlx::query_as::<_, SessionRecord>(
            "SELECT * FROM sessions WHERE id = ?"
        )
        .bind(session_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| super::error::Error::Database(e.to_string()))
    }

    /// 通过 Key 获取会话
    pub async fn get_session_by_key(
        &self,
        channel: &str,
        target_id: &str,
        sender_id: Option<&str>,
    ) -> Result<Option<SessionRecord>, super::error::Error> {
        let sender_id = sender_id.unwrap_or("");
        sqlx::query_as::<_, SessionRecord>(
            "SELECT * FROM sessions WHERE channel = ? AND target_id = ? AND sender_id = ?"
        )
        .bind(channel)
        .bind(target_id)
        .bind(sender_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| super::error::Error::Database(e.to_string()))
    }

    /// 创建会话
    pub async fn create_session(
        &self,
        session: &SessionRecord,
    ) -> Result<(), super::error::Error> {
        sqlx::query(
            "INSERT INTO sessions (id, channel, target_id, sender_id, agent_id, status, created_at, updated_at, metadata)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)"
        )
        .bind(&session.id)
        .bind(&session.channel)
        .bind(&session.target_id)
        .bind(&session.sender_id)
        .bind(&session.agent_id)
        .bind(&session.status)
        .bind(session.created_at)
        .bind(session.updated_at)
        .bind(&session.metadata)
        .execute(&self.pool)
        .await
        .map_err(|e| super::error::Error::Database(e.to_string()))?;

        Ok(())
    }

    /// 更新会话
    pub async fn update_session(
        &self,
        session: &SessionRecord,
    ) -> Result<bool, super::error::Error> {
        let result = sqlx::query(
            "UPDATE sessions SET 
                agent_id = ?, 
                status = ?, 
                updated_at = ?, 
                metadata = ?
             WHERE id = ?"
        )
        .bind(&session.agent_id)
        .bind(&session.status)
        .bind(session.updated_at)
        .bind(&session.metadata)
        .bind(&session.id)
        .execute(&self.pool)
        .await
        .map_err(|e| super::error::Error::Database(e.to_string()))?;

        Ok(result.rows_affected() > 0)
    }

    /// 删除会话
    pub async fn delete_session(
        &self,
        session_id: &str,
    ) -> Result<bool, super::error::Error> {
        // 先删除消息
        sqlx::query("DELETE FROM messages WHERE session_id = ?")
            .bind(session_id)
            .execute(&self.pool)
            .await
            .map_err(|e| super::error::Error::Database(e.to_string()))?;

        // 再删除会话
        let result = sqlx::query("DELETE FROM sessions WHERE id = ?")
            .bind(session_id)
            .execute(&self.pool)
            .await
            .map_err(|e| super::error::Error::Database(e.to_string()))?;

        Ok(result.rows_affected() > 0)
    }

    /// 获取会话消息
    pub async fn get_session_messages(
        &self,
        session_id: &str,
    ) -> Result<Vec<MessageRecord>, super::error::Error> {
        sqlx::query_as::<_, MessageRecord>(
            "SELECT * FROM messages WHERE session_id = ? ORDER BY created_at ASC"
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| super::error::Error::Database(e.to_string()))
    }

    /// 添加消息
    pub async fn add_message(
        &self,
        message: &MessageRecord,
    ) -> Result<(), super::error::Error> {
        sqlx::query(
            "INSERT INTO messages (id, session_id, role, content, created_at)
             VALUES (?, ?, ?, ?, ?)"
        )
        .bind(&message.id)
        .bind(&message.session_id)
        .bind(&message.role)
        .bind(&message.content)
        .bind(message.created_at)
        .execute(&self.pool)
        .await
        .map_err(|e| super::error::Error::Database(e.to_string()))?;

        // 更新会话时间
        sqlx::query(
            "UPDATE sessions SET updated_at = ? WHERE id = ?"
        )
        .bind(message.created_at)
        .bind(&message.session_id)
        .execute(&self.pool)
        .await
        .map_err(|e| super::error::Error::Database(e.to_string()))?;

        Ok(())
    }

    /// 获取配置
    pub async fn get_config(
        &self,
        key: &str,
    ) -> Result<Option<ConfigRecord>, super::error::Error> {
        sqlx::query_as::<_, ConfigRecord>(
            "SELECT * FROM configs WHERE key = ?"
        )
        .bind(key)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| super::error::Error::Database(e.to_string()))
    }

    /// 设置配置
    pub async fn set_config(
        &self,
        key: &str,
        value: &str,
    ) -> Result<(), super::error::Error> {
        let now = chrono::Utc::now().timestamp();

        sqlx::query(
            "INSERT OR REPLACE INTO configs (key, value, updated_at)
             VALUES (?, ?, ?)"
        )
        .bind(key)
        .bind(value)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| super::error::Error::Database(e.to_string()))?;

        Ok(())
    }

    /// 查询会话
    pub async fn query_sessions(
        &self,
        channel: Option<&str>,
        target_id: Option<&str>,
        sender_id: Option<&str>,
        agent_id: Option<&str>,
        status: Option<&str>,
        limit: usize,
    ) -> Result<Vec<SessionRecord>, super::error::Error> {
        use sqlx::Arguments;
        let mut sql = "SELECT * FROM sessions WHERE 1=1".to_string();
        let mut args = sqlx::sqlite::SqliteArguments::default();

        if let Some(c) = channel {
            sql.push_str(" AND channel = ?");
            args.add(c);
        }
        if let Some(t) = target_id {
            sql.push_str(" AND target_id = ?");
            args.add(t);
        }
        if let Some(s) = sender_id {
            sql.push_str(" AND sender_id = ?");
            args.add(s);
        }
        if let Some(a) = agent_id {
            sql.push_str(" AND agent_id = ?");
            args.add(a);
        }
        if let Some(s) = status {
            sql.push_str(" AND status = ?");
            args.add(s);
        }

        sql.push_str(" ORDER BY updated_at DESC LIMIT ?");
        args.add(limit as i64);

        sqlx::query_as_with::<_, SessionRecord, _>(&sql, args)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| super::error::Error::Database(e.to_string()))
    }

    /// 获取会话统计 (总数, 活跃数, 今日新增数)
    pub async fn get_session_stats(&self) -> Result<(i64, i64, i64), super::error::Error> {
        // 总会话数
        let total: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM sessions")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| super::error::Error::Database(e.to_string()))?;

        // 活跃会话数 (status='Active')
        let active: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM sessions WHERE status = 'Active'")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| super::error::Error::Database(e.to_string()))?;

        // 今日新增
        let now = chrono::Utc::now().timestamp_millis();
        let today_start = now - (now % 86400000);
        let today_new: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM sessions WHERE created_at >= ?")
            .bind(today_start)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| super::error::Error::Database(e.to_string()))?;

        Ok((total.0, active.0, today_new.0))
    }

    /// 清理过期会话
    pub async fn cleanup_expired_sessions(&self, expire_timestamp: i64) -> Result<u64, super::error::Error> {
        let mut tx = self.pool.begin().await.map_err(|e| super::error::Error::Database(e.to_string()))?;

        // 删除过期会话的消息
        sqlx::query("DELETE FROM messages WHERE session_id IN (SELECT id FROM sessions WHERE updated_at < ?)")
            .bind(expire_timestamp)
            .execute(&mut *tx)
            .await
            .map_err(|e| super::error::Error::Database(e.to_string()))?;

        // 删除过期会话
        let result = sqlx::query("DELETE FROM sessions WHERE updated_at < ?")
            .bind(expire_timestamp)
            .execute(&mut *tx)
            .await
            .map_err(|e| super::error::Error::Database(e.to_string()))?;
        
        tx.commit().await.map_err(|e| super::error::Error::Database(e.to_string()))?;

        Ok(result.rows_affected())
    }
}
