//! 审计日志存储模块
//!
//! 提供审计日志的存储功能，支持 SQLite 和文件存储

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::sync::Mutex;
use tracing::{error, info};

use sqlx::Row;
use crate::security::types::{AuditEvent, AuditLevel};

/// SQLite 表名
const TABLE_NAME: &str = "audit_logs";

/// 审计存储后端
///
/// 支持 SQLite 数据库存储和文件存储
#[derive(Clone)]
pub struct AuditStorage {
    /// SQLite 连接池
    pool: Arc<sqlx::SqlitePool>,
    /// 文件存储路径
    file_path: PathBuf,
    /// 序列化锁（防止并发写入文件）
    file_lock: Arc<Mutex<()>>,
}

impl AuditStorage {
    /// 创建审计存储
    ///
    /// 初始化 SQLite 连接和文件存储
    pub async fn new() -> Result<Self, sqlx::Error> {
        // 使用当前工作目录下的 audit 目录
        let current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let data_dir = current_dir.join("data/audit");

        // 确保目录存在
        if let Err(e) = tokio::fs::create_dir_all(&data_dir).await {
            tracing::error!(error = %e, path = %data_dir.display(), "创建审计目录失败");
        }

        let db_path = data_dir.join("audit.db");
        let file_path = data_dir.join("audit_logs.jsonl");

        tracing::info!(db_path = %db_path.display(), "尝试连接审计数据库");

        // 创建 SQLite 连接池
        let pool = sqlx::SqlitePool::connect(&db_path.display().to_string()).await?;

        // 初始化数据库表
        Self::init_schema(&pool).await?;

        Ok(Self {
            pool: Arc::new(pool),
            file_path,
            file_lock: Arc::new(Mutex::new(())),
        })
    }

    /// 初始化数据库表结构
    async fn init_schema(pool: &sqlx::SqlitePool) -> Result<(), sqlx::Error> {
        let query = format!(
            r#"
            CREATE TABLE IF NOT EXISTS {} (
                id TEXT PRIMARY KEY,
                timestamp INTEGER NOT NULL,
                level TEXT NOT NULL,
                category TEXT NOT NULL,
                event_type TEXT NOT NULL,
                message_id TEXT,
                channel TEXT,
                user_id TEXT,
                target_id TEXT,
                message_type TEXT,
                content TEXT,
                metadata TEXT
            )
            "#,
            TABLE_NAME
        );

        sqlx::query(&query).execute(pool).await?;

        // 创建索引
        let indexes = vec![
            format!(
                "CREATE INDEX IF NOT EXISTS idx_{}_timestamp ON {} (timestamp)",
                TABLE_NAME, TABLE_NAME
            ),
            format!(
                "CREATE INDEX IF NOT EXISTS idx_{}_level ON {} (level)",
                TABLE_NAME, TABLE_NAME
            ),
            format!(
                "CREATE INDEX IF NOT EXISTS idx_{}_category ON {} (category)",
                TABLE_NAME, TABLE_NAME
            ),
            format!(
                "CREATE INDEX IF NOT EXISTS idx_{}_user_id ON {} (user_id)",
                TABLE_NAME, TABLE_NAME
            ),
            format!(
                "CREATE INDEX IF NOT EXISTS idx_{}_channel ON {} (channel)",
                TABLE_NAME, TABLE_NAME
            ),
        ];

        for idx in indexes {
            sqlx::query(&idx).execute(pool).await.ok();
        }

        info!(table = TABLE_NAME, "审计日志表初始化完成");

        Ok(())
    }

    /// 存储审计事件
    ///
    /// # 参数说明
    /// * `event` - 审计事件
    pub async fn store(&self, event: &AuditEvent) -> Result<(), sqlx::Error> {
        // 1. 写入 SQLite
        let timestamp = event
            .timestamp
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        let metadata = serde_json::to_string(&event.metadata).unwrap_or_default();

        let query = format!(
            r#"
            INSERT INTO {} (
                id, timestamp, level, category, event_type,
                message_id, channel, user_id, target_id, message_type,
                content, metadata
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
            TABLE_NAME
        );

        sqlx::query(&query)
            .bind(&event.id)
            .bind(timestamp)
            .bind(format!("{:?}", event.level))
            .bind(format!("{:?}", event.category))
            .bind(&event.event_type)
            .bind(&event.context.message_id)
            .bind(&event.context.channel)
            .bind(&event.context.user_id)
            .bind(&event.context.target_id)
            .bind(&event.context.message_type)
            .bind(&event.content)
            .bind(&metadata)
            .execute(&*self.pool)
            .await?;

        // 2. 写入文件（JSONL 格式）
        self.append_to_file(event).await;

        Ok(())
    }

    /// 追加到文件（JSONL 格式）
    async fn append_to_file(&self, event: &AuditEvent) {
        let _guard = self.file_lock.lock().await;

        // 跳过原始内容（如果有）
        let file_event = FileAuditEvent {
            id: &event.id,
            timestamp: humantime::format_rfc3339(event.timestamp).to_string(),
            level: &event.level,
            category: &event.category,
            event_type: &event.event_type,
            context: &event.context,
            content: &event.content,
            metadata: &event.metadata,
        };

        if let Ok(line) = serde_json::to_string(&file_event) {
            if let Ok(mut file) = tokio::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.file_path)
                .await
            {
                let _ = tokio::io::AsyncWriteExt::write_all(&mut file, line.as_bytes()).await;
                let _ = tokio::io::AsyncWriteExt::write_all(&mut file, b"\n").await;
            }
        }
    }

    /// 查询审计事件
    ///
    /// # 参数说明
    /// * `start_time` - 开始时间
    /// * `end_time` - 结束时间
    /// * `level` - 级别过滤（可选）
    /// * `limit` - 最大返回数量
    ///
    /// # 返回值
    /// 匹配的审计事件列表（按时间倒序）
    pub async fn query(
        &self,
        start_time: SystemTime,
        end_time: SystemTime,
        level: Option<AuditLevel>,
        limit: Option<u32>,
    ) -> Result<Vec<AuditEvent>, sqlx::Error> {
        let start_ts = start_time
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        let end_ts = end_time
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(i64::MAX);

        let limit = limit.unwrap_or(100);

        // 构建查询（倒序排列，最近的在前）
        let query_str = format!(
            r#"SELECT id, timestamp, level, category, event_type,
                    message_id, channel, user_id, target_id, message_type,
                    content, metadata FROM {} WHERE timestamp BETWEEN ? AND ?"#,
            TABLE_NAME
        );

        // 根据是否需要级别过滤选择查询
        let rows = if let Some(lvl) = &level {
            // SQLite 不支持 order_by 方法，直接在 SQL 中添加 ORDER BY
            let query_with_level = format!("{} AND level = ? ORDER BY timestamp DESC LIMIT {}", query_str, limit);
            sqlx::query(&query_with_level)
                .bind(start_ts)
                .bind(end_ts)
                .bind(format!("{:?}", lvl))
                .fetch_all(&*self.pool)
                .await?
        } else {
            let query_no_level = format!("{} ORDER BY timestamp DESC LIMIT {}", query_str, limit);
            sqlx::query(&query_no_level)
                .bind(start_ts)
                .bind(end_ts)
                .fetch_all(&*self.pool)
                .await?
        };

        let mut events = Vec::new();

        for row in rows.iter() {
            let metadata_str: String = row.try_get(11)?;
            let metadata: serde_json::Value = serde_json::from_str(&metadata_str).unwrap_or_default();

            let event = AuditEvent {
                id: row.try_get(0)?,
                timestamp: SystemTime::UNIX_EPOCH + Duration::from_secs(row.try_get::<i64, _>(1)? as u64),
                level: serde_json::from_str(&row.try_get::<String, _>(2)?).unwrap_or(AuditLevel::Info),
                category: serde_json::from_str(&row.try_get::<String, _>(3)?).unwrap_or(crate::security::types::AuditCategory::System),
                event_type: row.try_get(4)?,
                context: crate::security::types::AuditContext {
                    message_id: row.try_get(5)?,
                    channel: row.try_get(6)?,
                    user_id: row.try_get(7)?,
                    target_id: row.try_get(8)?,
                    message_type: row.try_get(9)?,
                    client_ip: None,
                    session_id: None,
                    agent_id: None,
                    ai_provider: None,
                    ai_model: None,
                    prompt_tokens: None,
                    completion_tokens: None,
                    duration_ms: None,
                    routing_confidence: None,
                    is_safe: true,
                    detected_words: vec![],
                },
                content: row.try_get(10)?,
                original_content: None,
                metadata,
            };

            events.push(event);
        }

        Ok(events)
    }

    /// 查询审计统计
    pub async fn get_stats(&self) -> Result<crate::security::types::AuditStats, sqlx::Error> {
        // 总事件数
        let total: i64 = sqlx::query_scalar(format!("SELECT COUNT(*) FROM {}", TABLE_NAME).as_str())
            .fetch_one(&*self.pool)
            .await?;

        // 今日事件数
        let today_start = chrono::Utc::now()
            .naive_utc()
            .date()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .timestamp() as i64;
        let today: i64 = sqlx::query_scalar(format!(
            "SELECT COUNT(*) FROM {} WHERE timestamp >= {}",
            TABLE_NAME, today_start
        )
        .as_str())
        .fetch_one(&*self.pool)
        .await?;

        // 警告和错误数
        let warning: i64 = sqlx::query_scalar(format!(
            "SELECT COUNT(*) FROM {} WHERE level = 'WARNING'",
            TABLE_NAME
        )
        .as_str())
        .fetch_one(&*self.pool)
        .await?;

        let error_count: i64 = sqlx::query_scalar(format!(
            "SELECT COUNT(*) FROM {} WHERE level = 'ERROR'",
            TABLE_NAME
        )
        .as_str())
        .fetch_one(&*self.pool)
        .await?;

        Ok(crate::security::types::AuditStats {
            total_events: total as u64,
            by_level: serde_json::json!({}),
            by_category: serde_json::json!({}),
            by_channel: serde_json::json!({}),
            today_events: today as u64,
            warning_events: warning as u64,
            error_events: error_count as u64,
        })
    }

    /// 删除指定时间之前的审计日志
    ///
    /// # 参数说明
    /// * `before` - 删除此时间点之前的数据
    ///
    /// # 返回值
    /// 删除的事件数量
    pub async fn delete_before(&self, before: SystemTime) -> Result<u64, sqlx::Error> {
        let before_ts = before
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        let query_str = format!("DELETE FROM {} WHERE timestamp < ?", TABLE_NAME);
        let result = sqlx::query(&query_str)
            .bind(before_ts)
            .execute(&*self.pool)
            .await?;

        Ok(result.rows_affected() as u64)
    }
}

/// 用于文件存储的审计事件（不包含原始内容）
#[derive(serde::Serialize)]
struct FileAuditEvent<'a> {
    id: &'a String,
    timestamp: String,
    level: &'a AuditLevel,
    category: &'a crate::security::types::AuditCategory,
    event_type: &'a String,
    context: &'a crate::security::types::AuditContext,
    content: &'a String,
    metadata: &'a serde_json::Value,
}
