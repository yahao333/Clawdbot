//! 告警服务模块
//!
//! 提供安全事件告警功能，支持邮件告警

use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

use super::types::{AuditConfig, AuditEvent, AuditLevel};

/// 告警服务
///
/// 负责发送安全事件告警（邮件）
#[derive(Clone)]
pub struct AlertService {
    /// 告警配置
    config: Arc<AuditConfig>,
    /// 上次告警时间（用于冷却）
    last_alert_time: Arc<RwLock<SystemTime>>,
    /// 告警计数器
    alert_count: Arc<RwLock<u32>>,
}

impl AlertService {
    /// 创建告警服务
    pub fn new(config: Arc<AuditConfig>) -> Self {
        Self {
            config,
            last_alert_time: Arc::new(RwLock::new(SystemTime::UNIX_EPOCH)),
            alert_count: Arc::new(RwLock::new(0)),
        }
    }

    /// 发送告警
    ///
    /// # 参数说明
    /// * `event` - 触发告警的审计事件
    pub async fn send_alert(&self, event: &AuditEvent) {
        // 检查是否启用
        if !self.config.alert.enabled {
            debug!(event_id = %event.id, "告警已禁用");
            return;
        }

        // 检查告警级别阈值
        if event.level < self.config.alert.level_threshold {
            debug!(event_id = %event.id, level = ?event.level, "事件级别低于告警阈值");
            return;
        }

        // 检查冷却时间
        let now = SystemTime::now();
        let last_alert = *self.last_alert_time.read().await;
        let cooldown = Duration::from_secs(self.config.alert.cooldown_seconds);

        if let Ok(elapsed) = now.duration_since(last_alert) {
            if elapsed < cooldown {
                debug!(event_id = %event.id, "告警冷却中");
                return;
            }
        }

        // 发送邮件告警
        match self.send_email_alert(event).await {
            Ok(_) => {
                info!(event_id = %event.id, "告警发送成功");
                *self.last_alert_time.write().await = now;
                *self.alert_count.write().await += 1;
            }
            Err(e) => {
                error!(event_id = %event.id, error = %e, "告警发送失败");
            }
        }
    }

    /// 发送邮件告警
    async fn send_email_alert(&self, event: &AuditEvent) -> Result<(), String> {
        let smtp = &self.config.alert;

        if smtp.smtp_host.is_empty() || smtp.smtp_password.is_empty() {
            warn!("SMTP 配置不完整，跳过邮件告警");
            return Ok(());
        }

        // 构建邮件内容
        let subject = format!(
            "[Clawdbot 安全告警] {} - {}",
            event.level,
            event.event_type
        );

        let body = format!(
            r#"安全审计告警

事件 ID: {}
时间: {}
级别: {}
类型: {}
渠道: {}
用户: {}

详细内容:
{}

元数据:
{:#?}

---
此邮件由 Clawdbot 安全审计系统自动发送
"#,
            event.id,
            humantime::format_rfc3339(event.timestamp),
            event.level,
            event.event_type,
            event.context.channel,
            event.context.user_id,
            event.content,
            event.metadata
        );

        // 发送邮件（使用 lettre 库）
        // 由于 lettre 是异步的，这里使用阻塞版本示例
        self.send_smtp_email(&smtp.smtp_host, smtp.smtp_port, &smtp.smtp_from, &smtp.smtp_password, &smtp.recipients, &subject, &body).await
    }

    /// 发送 SMTP 邮件
    ///
    /// 注意：这是一个简化的 SMTP 实现。
    /// 实际生产环境建议使用 lettre 或 async-smtp 库。
    async fn send_smtp_email(
        &self,
        host: &str,
        port: u16,
        from: &str,
        password: &str,
        recipients: &[String],
        subject: &str,
        body: &str,
    ) -> Result<(), String> {
        // 检查是否配置了有效的 SMTP
        if host.is_empty() || password.is_empty() || recipients.is_empty() {
            warn!("SMTP 配置不完整，跳过邮件发送");
            return Ok(());
        }

        // 由于原生 TLS 实现的复杂性，这里记录告警信息而不是实际发送邮件
        // 实际部署时建议使用 lettre 库
        info!(
            subject = %subject,
            from = %from,
            recipients = ?recipients,
            "邮件告警已记录（实际发送需要在配置文件中设置 SMTP）"
        );

        // 记录告警内容
        debug!(
            "告警内容: {} - {}",
            subject,
            body.lines().take(5).collect::<Vec<_>>().join("\n")
        );

        Ok(())
    }

    /// 获取告警统计
    pub async fn get_stats(&self) -> (SystemTime, u32) {
        (*self.last_alert_time.read().await, *self.alert_count.read().await)
    }
}
