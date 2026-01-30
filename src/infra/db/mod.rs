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
use tracing::{debug, info};

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
        let pool = SqlitePool::connect(&url)
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
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            )
        "#).execute(pool).await
            .map_err(|e| super::error::Error::Database(e.to_string()))?;

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
    /// 创建时间
    pub created_at: i64,
    /// 更新时间
    pub updated_at: i64,
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

    /// 创建会话
    pub async fn create_session(
        &self,
        session: &SessionRecord,
    ) -> Result<(), super::error::Error> {
        sqlx::query(
            "INSERT INTO sessions (id, channel, target_id, sender_id, agent_id, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)"
        )
        .bind(&session.id)
        .bind(&session.channel)
        .bind(&session.target_id)
        .bind(&session.sender_id)
        .bind(&session.agent_id)
        .bind(session.created_at)
        .bind(session.updated_at)
        .execute(&self.pool)
        .await
        .map_err(|e| super::error::Error::Database(e.to_string()))?;

        Ok(())
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
}
