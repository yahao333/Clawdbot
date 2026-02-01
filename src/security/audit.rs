//! 安全审计模块
//!
//! 提供完整的安全审计功能，包括：
//! - 消息内容审计
//! - API 调用审计
//! - 用户行为审计
//! - 安全事件告警
//!
//! # 使用示例
//! ```rust
//! use clawdbot::security::AuditService;
//!
//! // 创建审计服务
//! let audit_service = AuditService::new(config).await?;
//!
//! // 记录消息
//! audit_service.log_message(&context, &message, true).await?;
//!
//! // 记录 API 调用
//! audit_service.log_api_call("minimax", "chat", 150, true, &context).await?;
//! ```

use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};
use serde::{Deserialize, Serialize};

use crate::infra::error::{Error, Result};
use crate::infra::config::Config;
use crate::security::types::{AuditEvent, AuditLevel, AuditCategory, AuditContext, AuditStats};
use crate::security::storage::AuditStorage;
use crate::security::alert::AlertService;

/// 审计服务
///
/// 统一的安全审计入口，负责收集、存储和转发审计事件
#[derive(Clone)]
pub struct AuditService {
    /// 审计存储后端
    storage: Arc<AuditStorage>,
    /// 告警服务
    alert_service: Arc<AlertService>,
    /// 敏感词列表
    sensitive_words: Arc<RwLock<Vec<String>>>,
    /// 告警阈值配置
    config: Arc<Config>,
}

impl AuditService {
    /// 创建审计服务
    ///
    /// # 参数说明
    /// * `config` - 应用配置（包含审计配置）
    ///
    /// # 返回值
    /// 创建的审计服务实例
    pub async fn new(config: Arc<Config>) -> Result<Self> {
        let storage = Arc::new(AuditStorage::new().await?);

        // 先克隆 security 配置以避免 Arc 移动问题
        let security_config = (*config).security.clone();

        // 从 Config.security 转换到 AuditConfig
        let audit_config = Arc::new(crate::security::types::AuditConfig {
            enabled: security_config.enabled,
            audit_store_original: security_config.audit_store_original,
            audit_retention_days: security_config.audit_retention_days.unwrap_or(30),
            alert: crate::security::types::AlertConfig {
                enabled: security_config.alert_enabled,
                level_threshold: crate::security::types::AuditLevel::Warning,
                smtp_host: security_config.smtp_server.unwrap_or_default(),
                smtp_port: security_config.smtp_port.unwrap_or(587),
                smtp_from: security_config.from_address.unwrap_or_default(),
                smtp_password: security_config.smtp_password.unwrap_or_default(),
                recipients: security_config.to_addresses.unwrap_or_default(),
                cooldown_seconds: 300,
            },
        });

        let alert_service = Arc::new(AlertService::new(audit_config));
        let sensitive_words = Arc::new(RwLock::new(Self::default_sensitive_words()));

        Ok(Self {
            storage,
            alert_service,
            sensitive_words,
            config,
        })
    }

    /// 默认敏感词列表
    ///
    /// 返回内置的敏感词检测列表
    fn default_sensitive_words() -> Vec<String> {
        vec![
            "password".to_string(),
            "pwd".to_string(),
            "secret".to_string(),
            "token".to_string(),
            "api_key".to_string(),
            "apikey".to_string(),
            "access_key".to_string(),
            "private_key".to_string(),
            "credential".to_string(),
            "密码".to_string(),
            "密钥".to_string(),
            "令牌".to_string(),
        ]
    }

    /// 记录消息审计事件
    ///
    /// 记录进站和出站消息，进行内容审计
    ///
    /// # 参数说明
    /// * `context` - 审计上下文（包含消息信息）
    /// * `content` - 消息内容
    /// * `is_inbound` - 是否是入站消息
    ///
    /// # 返回值
    /// 审计事件 ID
    pub async fn log_message(
        &self,
        context: &AuditContext,
        content: &str,
        is_inbound: bool,
    ) -> Result<String> {
        let event_id = format!("msg_{}_{}", context.channel, uuid::Uuid::new_v4());

        // 内容安全检测
        let (is_safe, detected_words) = self.check_content_safety(content).await;

        // 脱敏处理
        let masked_content = self.mask_sensitive_content(content).await;

        // 构建审计事件
        let event = AuditEvent {
            id: event_id.clone(),
            timestamp: SystemTime::now(),
            level: if is_safe { AuditLevel::Info } else { AuditLevel::Warning },
            category: if is_inbound { AuditCategory::InboundMessage } else { AuditCategory::OutboundMessage },
            event_type: "message".to_string(),
            context: context.clone(),
            content: masked_content.clone(),
            original_content: if self.config.security.audit_store_original {
                Some(content.to_string())
            } else {
                None
            },
            metadata: serde_json::json!({
                "is_safe": is_safe,
                "detected_words": detected_words,
                "is_inbound": is_inbound,
                // 额外上下文信息
                "session_id": context.session_id,
                "agent_id": context.agent_id,
                "ai_provider": context.ai_provider,
                "ai_model": context.ai_model,
                "prompt_tokens": context.prompt_tokens,
                "completion_tokens": context.completion_tokens,
                "total_tokens": context.prompt_tokens.map(|p| p + context.completion_tokens.unwrap_or(0)),
                "duration_ms": context.duration_ms,
                "routing_confidence": context.routing_confidence,
            }),
        };

        // 存储审计事件
        self.storage.store(&event).await?;

        // 如果不安全，发送告警
        if !is_safe {
            self.alert_service.send_alert(&event).await;
        }

        info!(event_id = %event_id, is_safe = is_safe, "消息审计完成");

        Ok(event_id)
    }

    /// 记录 API 调用审计事件
    ///
    /// 记录 AI API 和第三方 API 的调用情况
    ///
    /// # 参数说明
    /// * `provider` - API 提供者名称（如：minimax, openai）
    /// * `action` - API 操作名称（如：chat, embeddings）
    /// * `latency_ms` - 调用延迟（毫秒）
    /// * `success` - 是否成功
    /// * `context` - 审计上下文
    pub async fn log_api_call(
        &self,
        provider: &str,
        action: &str,
        latency_ms: u64,
        success: bool,
        context: &AuditContext,
    ) -> Result<String> {
        let event_id = format!("api_{}_{}", provider, uuid::Uuid::new_v4());

        let event = AuditEvent {
            id: event_id.clone(),
            timestamp: SystemTime::now(),
            level: if success { AuditLevel::Info } else { AuditLevel::Error },
            category: AuditCategory::ApiCall,
            event_type: format!("{}.{}", provider, action),
            context: context.clone(),
            content: format!("API 调用: {} -> {}", provider, action),
            original_content: None,
            metadata: serde_json::json!({
                "provider": provider,
                "action": action,
                "latency_ms": latency_ms,
                "success": success,
            }),
        };

        self.storage.store(&event).await?;

        // 如果失败，发送告警
        if !success {
            self.alert_service.send_alert(&event).await;
        }

        debug!(event_id = %event_id, provider = %provider, action = %action, "API 审计完成");

        Ok(event_id)
    }

    /// 记录用户行为审计事件
    ///
    /// 记录用户登录、消息频率异常等行为
    ///
    /// # 参数说明
    /// * `user_id` - 用户 ID
    /// * `action` - 用户操作（如：login, logout, message）
    /// * `context` - 审计上下文
    pub async fn log_user_action(
        &self,
        user_id: &str,
        action: &str,
        context: &AuditContext,
    ) -> Result<String> {
        let event_id = format!("user_{}_{}", action, uuid::Uuid::new_v4());

        let event = AuditEvent {
            id: event_id.clone(),
            timestamp: SystemTime::now(),
            level: AuditLevel::Info,
            category: AuditCategory::UserAction,
            event_type: action.to_string(),
            context: context.clone(),
            content: format!("用户操作: {} -> {}", user_id, action),
            original_content: None,
            metadata: serde_json::json!({
                "user_id": user_id,
                "action": action,
            }),
        };

        self.storage.store(&event).await?;

        info!(event_id = %event_id, user_id = %user_id, action = %action, "用户行为审计完成");

        Ok(event_id)
    }

    /// 检查内容安全性
    ///
    /// 检测内容中是否包含敏感词
    ///
    /// # 返回值
    /// (是否安全, 检测到的敏感词列表)
    async fn check_content_safety(&self, content: &str) -> (bool, Vec<String>) {
        let words = self.sensitive_words.read().await;
        let mut detected = Vec::new();

        for word in words.iter() {
            if content.to_lowercase().contains(&word.to_lowercase()) {
                detected.push(word.clone());
            }
        }

        (detected.is_empty(), detected)
    }

    /// 脱敏敏感内容
    ///
    /// 将敏感信息替换为掩码
    async fn mask_sensitive_content(&self, content: &str) -> String {
        let words = self.sensitive_words.read().await;
        let mut masked = content.to_string();

        for word in words.iter() {
            // 替换模式：敏感词 -> [脱敏]
            let pattern = format!("(?i){}", regex::escape(word));
            let re = regex::RegexBuilder::new(&pattern)
                .case_insensitive(true)
                .build()
                .unwrap();

            let replacement = re.replace_all(&masked, "[脱敏]");
            masked = replacement.to_string();
        }

        masked
    }

    /// 导出审计日志
    ///
    /// 将审计日志导出为 JSON 格式
    ///
    /// # 参数说明
    /// * `start_time` - 开始时间
    /// * `end_time` - 结束时间
    /// * `level` - 过滤日志级别（可选）
    ///
    /// # 返回值
    /// 导出 JSON 字符串
    pub async fn export_logs(
        &self,
        start_time: SystemTime,
        end_time: SystemTime,
        level: Option<AuditLevel>,
    ) -> Result<String> {
        let events = self.storage.query(start_time, end_time, level, None).await?;

        // 序列化时脱敏，包含所有元数据字段
        let masked_events: Vec<serde_json::Value> = events.iter()
            .map(|e| {
                serde_json::json!({
                    "id": e.id,
                    "timestamp": humantime::format_rfc3339(e.timestamp).to_string(),
                    "level": format!("{:?}", e.level),
                    "category": format!("{:?}", e.category),
                    "event_type": e.event_type,
                    // 上下文信息
                    "message_id": e.context.message_id,
                    "channel": e.context.channel,
                    "user_id": e.context.user_id,
                    "target_id": e.context.target_id,
                    "message_type": e.context.message_type,
                    // 会话和AI信息
                    "session_id": e.context.session_id,
                    "agent_id": e.context.agent_id,
                    "ai_provider": e.context.ai_provider,
                    "ai_model": e.context.ai_model,
                    "prompt_tokens": e.context.prompt_tokens,
                    "completion_tokens": e.context.completion_tokens,
                    "duration_ms": e.context.duration_ms,
                    "routing_confidence": e.context.routing_confidence,
                    // 安全信息
                    "is_safe": e.context.is_safe,
                    "detected_words": e.context.detected_words,
                    // 内容
                    "content": e.content,
                    // 完整元数据
                    "metadata": e.metadata,
                })
            })
            .collect();

        serde_json::to_string_pretty(&masked_events)
            .map_err(|e| Error::Serialization(e.to_string()))
    }

    /// 清理过期审计日志
    ///
    /// 清理超过保留期限（默认 30 天）的审计日志
    pub async fn cleanup_expired_logs(&self) -> Result<u64> {
        let retention_days = self.config.security.audit_retention_days.unwrap_or(30);
        let cutoff = SystemTime::now() - Duration::from_secs(60 * 60 * 24 * retention_days as u64);

        let count = self.storage.delete_before(cutoff).await?;
        info!(deleted_count = count, "清理过期审计日志完成");

        Ok(count)
    }

    /// 获取审计统计
    pub async fn get_stats(&self) -> Result<AuditStats> {
        self.storage.get_stats().await.map_err(|e| Error::Database(e.to_string()))
    }
}
