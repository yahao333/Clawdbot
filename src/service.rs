//! 服务模块
//!
//! 负责机器人的完整生命周期管理。

use std::sync::Arc;
use std::time::Duration;
use tokio::signal;
use tokio::sync::broadcast;
use tracing::{info, warn};

use crate::channels::feishu::{FeishuClient, FeishuCredentials, FeishuWsMonitor, MessageEventHandler, FeishuMessageSender};
use crate::core::message::{HandlerContext, MessageQueue, DefaultMessageHandler, UnifiedMessageSender, queue::QueueConfig};
use crate::core::routing::DefaultRouter;
use crate::core::agent::{AiEngine, DefaultAiEngine};
use crate::core::session::SessionManager;
use crate::infra::config::{Config, ConfigLoader};

/// 服务状态
#[derive(Debug, Clone, PartialEq)]
pub enum ServiceStatus {
    Initializing,
    Running,
    Stopping,
    Stopped,
    Error(String),
}

/// 服务配置
#[derive(Debug, Clone)]
pub struct ServiceConfig {
    pub config_path: String,
    pub verbose: bool,
    pub port: u16,
    pub health_check_interval: Duration,
}

impl Default for ServiceConfig {
    fn default() -> Self {
        Self {
            config_path: "clawdbot.toml".to_string(),
            verbose: false,
            port: 8080,
            health_check_interval: Duration::from_secs(30),
        }
    }
}

/// Clawdbot 服务
#[derive(Clone)]
pub struct ClawdbotService {
    config: ServiceConfig,
    status: Arc<tokio::sync::RwLock<ServiceStatus>>,
    shutdown_tx: broadcast::Sender<()>,
    /// 加载的配置
    loaded_config: Arc<Option<Config>>,
    /// Web 状态（用于仪表盘数据更新）
    pub web_state: Arc<crate::web::WebState>,
}

impl ClawdbotService {
    pub fn new(config: ServiceConfig, web_state: Arc<crate::web::WebState>) -> Self {
        let (shutdown_tx, _) = broadcast::channel(1);

        Self {
            config,
            status: Arc::new(tokio::sync::RwLock::new(ServiceStatus::Initializing)),
            shutdown_tx,
            loaded_config: Arc::new(None),
            web_state,
        }
    }

    pub async fn start(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        info!("开始启动服务...");

        *self.status.write().await = ServiceStatus::Running;

        // 获取配置
        let config = self.loaded_config.as_ref().clone().unwrap_or_default();
        let feishu_config = config.channels.get("feishu");

        // 如果启用了飞书渠道，启动飞书消息监控
        if let Some(channel_config) = feishu_config {
            if channel_config.enabled {
                self.start_feishu_monitor(&config).await?;
            }
        }

        // 启动关闭信号监听
        let mut rx = self.shutdown_tx.subscribe();
        let shutdown_tx = self.shutdown_tx.clone();
        tokio::spawn(async move {
            let _ = signal::ctrl_c().await;
            warn!("收到 Ctrl+C 信号，准备关闭服务...");
            let _ = shutdown_tx.send(());
        });

        // 等待关闭信号
        let _ = rx.recv().await;

        *self.status.write().await = ServiceStatus::Stopped;
        info!("服务已停止");

        Ok(())
    }

    /// 启动飞书消息监控
    async fn start_feishu_monitor(&self, config: &Config) -> Result<(), Box<dyn std::error::Error>> {
        info!("启动飞书消息监控服务...");

        // 获取飞书凭证
        let feishu_config = config.channels.get("feishu")
            .ok_or_else(|| Box::new(std::io::Error::new(std::io::ErrorKind::NotFound, "飞书配置不存在")) as Box<dyn std::error::Error>)?;

        let app_id = feishu_config.credentials.get("app_id")
            .ok_or_else(|| Box::new(std::io::Error::new(std::io::ErrorKind::NotFound, "app_id 不存在")) as Box<dyn std::error::Error>)?;
        let app_secret = feishu_config.credentials.get("app_secret")
            .ok_or_else(|| Box::new(std::io::Error::new(std::io::ErrorKind::NotFound, "app_secret 不存在")) as Box<dyn std::error::Error>)?;

        // 创建飞书客户端
        let credentials = FeishuCredentials {
            app_id: app_id.clone(),
            app_secret: app_secret.clone(),
            verification_token: None,
            encrypt_key: None,
        };
        let feishu_client = Arc::new(FeishuClient::new(credentials));

        // 创建消息处理器组件
        let router: Arc<dyn crate::core::routing::Router> = Arc::new(DefaultRouter::new("default"));
        let queue_config = QueueConfig::default();
        let message_queue = Arc::new(MessageQueue::new(queue_config));
        let ai_engine: Arc<dyn AiEngine> = Arc::new(DefaultAiEngine::new(config));
        // 创建会话管理器
        let session_config = crate::core::session::SessionConfig {
            expire_seconds: config.session.expire_seconds.unwrap_or(3600),
            max_history: config.session.max_history.unwrap_or(50),
            persist_enabled: config.session.persist_enabled.unwrap_or(true),
            db_path: config.session.db_path.clone().unwrap_or_else(|| "data/clawdbot.db".to_string()),
        };
        let session_manager = Arc::new(SessionManager::new(Some(session_config)).await);
        // 数据库初始化暂时跳过
        // let database = Arc::new(Database::new("data/clawdbot.db").await?);

        // 创建统一消息发送器
        let sender = Arc::new(UnifiedMessageSender::new());
        // 注册飞书发送器
        let feishu_sender = Arc::new(FeishuMessageSender::from_client((*feishu_client).clone()));
        sender.register("feishu", feishu_sender).await;

        // 创建 HandlerContext（带 Web 状态）
        let handler_context = HandlerContext::new_with_web_state(
            Arc::new(config.clone()),
            message_queue.clone(),
            router.clone(),
            ai_engine.clone(),
            sender,
            session_manager,
            self.web_state.clone(),
        );

        // 更新飞书渠道状态为已连接
        self.web_state.update_channel_status("feishu", crate::web::ChannelConnectionStatus {
            connected: true,
            last_connected: Some(chrono::Utc::now().to_rfc3339()),
            last_error: None,
            message_count: 0,
        }).await;

        // 启动消息队列处理循环
        let ctx_clone = handler_context.clone();
        message_queue.start_processing(move |msg| {
            let ctx = ctx_clone.clone();
            async move {
                DefaultMessageHandler::process_message(ctx, msg).await;
            }
        });

        let message_handler: Arc<dyn crate::core::message::MessageHandler> = Arc::new(DefaultMessageHandler::default());

        // 创建事件处理器
        let event_handler = MessageEventHandler::new();

        // 创建飞书 WebSocket 监控器
        let monitor = FeishuWsMonitor::new(
            feishu_client.clone(),
            event_handler,
            handler_context.clone(),
            message_handler,
        );

        // 启动监控服务
        tokio::spawn(async move {
            if let Err(e) = monitor.start().await {
                tracing::error!(error = %e, "飞书消息监控服务启动失败");
            }
        });

        info!("飞书消息监控服务已启动（WebSocket 长链接）");

        Ok(())
    }

    pub async fn stop(&mut self) {
        info!("正在停止服务...");

        *self.status.write().await = ServiceStatus::Stopping;

        let _ = self.shutdown_tx.send(());

        info!("停止信号已发送");
    }

    pub async fn initialize(&mut self, config_path: &str) -> Result<(), Box<dyn std::error::Error>> {
        info!(path = config_path, "初始化服务...");

        // 1. 加载配置
        let config = self.load_config(config_path).await?;
        self.loaded_config = Arc::new(Some(config));

        info!("服务初始化完成");
        Ok(())
    }

    async fn load_config(&mut self, config_path: &str) -> Result<Config, Box<dyn std::error::Error>> {
        info!(path = config_path, "加载配置文件");

        let loader = ConfigLoader::new();
        let config = loader.load(config_path)
            .await
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;

        info!("配置加载成功");
        Ok(config)
    }

    pub async fn status(&self) -> ServiceStatus {
        self.status.read().await.clone()
    }
}

impl Default for ClawdbotService {
    fn default() -> Self {
        Self::new(ServiceConfig::default(), Arc::new(crate::web::WebState::new()))
    }
}
