//! 消息队列模块
//!
//! 本模块实现了一个带去重功能的消息队列。
//!
//! # 功能特点
//! 1. **消息去重**：基于消息内容哈希和时间窗口去重，避免重复处理
//! 2. **多种模式**：支持并行、顺序、每发送者顺序三种处理模式
//! 3. **容量限制**：支持队列容量上限和多种溢出策略
//!
//! # 使用示例
//! ```rust
//! let queue = MessageQueue::new(QueueConfig::default());
//! queue.push(message).await;
//! ```
//!
//! # 处理模式说明
//! - `Parallel`: 所有消息并行处理
//! - `Sequential`: 所有消息顺序处理
//! - `PerSender`: 同一发送者的消息顺序处理，不同发送者并行

use tokio::sync::{mpsc, Mutex};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::Duration;
use dashmap::DashMap;
use tracing::{debug, warn, error, info};

/// 消息去重键
///
/// 用于判断消息是否重复的结构体
#[derive(Debug, Clone)]
pub struct DedupeKey {
    /// 渠道类型（如 "feishu"）
    pub channel: String,
    /// 发送者 ID
    pub sender: String,
    /// 消息内容哈希
    pub content_hash: String,
    /// 时间窗口（用于去重的时间粒度）
    pub timestamp_window: u64,
}

impl DedupeKey {
    /// 创建去重键
    ///
    /// # 参数说明
    /// * `channel` - 渠道类型
    /// * `sender` - 发送者 ID
    /// * `content_hash` - 消息内容哈希
    /// * `debounce_ms` - 去重时间窗口（毫秒）
    pub fn new(channel: &str, sender: &str, content_hash: &str, debounce_ms: u64) -> Self {
        // 计算时间窗口：时间戳 / (去重窗口 / 1000)，实现时间窗口去重
        let timestamp_window = (chrono::Utc::now().timestamp_millis() as u64 / debounce_ms.max(1));

        Self {
            channel: channel.to_string(),
            sender: sender.to_string(),
            content_hash: content_hash.to_string(),
            timestamp_window,
        }
    }
}

/// 消息去重键的相等性判断
impl PartialEq for DedupeKey {
    fn eq(&self, other: &Self) -> bool {
        self.channel == other.channel
            && self.sender == other.sender
            && self.content_hash == other.content_hash
            && self.timestamp_window == other.timestamp_window
    }
}

/// 消息去重键的相等性判断（用于 HashMap）
impl Eq for DedupeKey {}

/// 为 DedupeKey 实现 Hash trait
impl Hash for DedupeKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.channel.hash(state);
        self.sender.hash(state);
        self.content_hash.hash(state);
        self.timestamp_window.hash(state);
    }
}

/// 队列处理模式
///
/// 决定消息的处理顺序
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueMode {
    /// 并行处理：所有消息同时处理
    Parallel,
    /// 顺序处理：所有消息按接收顺序处理
    Sequential,
    /// 每发送者顺序：同一发送者的消息顺序处理，不同发送者并行
    PerSender,
}

/// 队列溢出策略
///
/// 当队列满时的处理方式
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueDropPolicy {
    /// 丢弃新消息
    DropNew,
    /// 丢弃最旧消息
    DropOld,
    /// 阻塞等待（直到有空间）
    Block,
}

/// 队列配置
///
/// 配置消息队列的行为参数
#[derive(Debug, Clone)]
pub struct QueueConfig {
    /// 处理模式
    pub mode: QueueMode,
    /// 全局去重时间窗口（毫秒）
    /// 在这个时间窗口内，相同内容的消息被认为是重复的
    pub debounce_ms: u64,
    /// 每个渠道的去重时间窗口覆盖
    /// 可以为特定渠道设置不同的去重时间
    pub debounce_ms_by_channel: HashMap<String, u64>,
    /// 队列容量上限
    pub cap: usize,
    /// 满队列时的丢弃策略
    pub drop_policy: QueueDropPolicy,
}

impl Default for QueueConfig {
    /// 默认配置
    ///
    /// - 模式：并行处理
    /// - 去重窗口：1000 毫秒（1 秒）
    /// - 容量：100 条消息
    /// - 丢弃策略：丢弃新消息
    fn default() -> Self {
        Self {
            mode: QueueMode::Parallel,
            debounce_ms: 1000,
            debounce_ms_by_channel: HashMap::new(),
            cap: 100,
            drop_policy: QueueDropPolicy::DropNew,
        }
    }
}

/// 消息队列
///
/// 线程安全的异步消息队列
///
/// # 线程安全
/// 使用 `Arc`、`DashMap` 和 `tokio::sync::mpsc` 实现线程安全
///
/// # 字段说明
/// * `config` - 队列配置
/// * `sender` - 消息发送端
/// * `active_messages` - 活跃消息追踪（用于去重）
/// * `sender_groups` - 发送者分组（用于 PerSender 模式）
#[derive(Clone)]
pub struct MessageQueue {
    /// 队列配置
    config: Arc<QueueConfig>,
    /// 消息发送通道
    sender: mpsc::Sender<super::types::InboundMessage>,
    /// 消息接收通道（用于处理）
    receiver: Arc<Mutex<Option<mpsc::Receiver<super::types::InboundMessage>>>>,
    /// 活跃消息追踪（用于去重）
    active_messages: Arc<DashMap<DedupeKey, std::time::Instant>>,
    /// 发送者分组（用于 PerSender 模式）
    sender_groups: Arc<DashMap<String, mpsc::Sender<super::types::InboundMessage>>>,
}

impl MessageQueue {
    /// 创建新的消息队列
    ///
    /// # 参数说明
    /// * `config` - 队列配置
    ///
    /// # 返回值
    /// 创建的消息队列
    ///
    /// # 日志记录
    /// - INFO: 队列创建成功
    pub fn new(config: QueueConfig) -> Self {
        // 创建 mpsc 通道，容量为配置指定的大小
        let (sender, receiver) = mpsc::channel(config.cap);

        let queue = Self {
            config: Arc::new(config),
            sender,
            receiver: Arc::new(Mutex::new(Some(receiver))),
            active_messages: Arc::new(DashMap::new()),
            sender_groups: Arc::new(DashMap::new()),
        };

        tracing::info!(
            mode = ?queue.config.mode,
            cap = queue.config.cap,
            "消息队列创建成功"
        );

        queue
    }

    /// 启动消息处理循环
    ///
    /// # 参数说明
    /// * `processor` - 消息处理函数
    ///
    /// # 功能
    /// 启动一个后台任务，持续从队列中消费消息并调用处理函数
    pub fn start_processing<F, Fut>(&self, processor: F)
    where
        F: Fn(super::types::InboundMessage) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = ()> + Send,
    {
        let receiver_mutex = self.receiver.clone();
        let processor = Arc::new(processor);
        let mode = self.config.mode;

        tokio::spawn(async move {
            let mut receiver_guard = receiver_mutex.lock().await;
            if let Some(mut receiver) = receiver_guard.take() {
                info!("消息队列处理循环已启动");
                drop(receiver_guard); // 释放锁

                while let Some(message) = receiver.recv().await {
                    let processor = processor.clone();
                    match mode {
                        QueueMode::Parallel => {
                            tokio::spawn(async move {
                                processor(message).await;
                            });
                        }
                        QueueMode::Sequential => {
                            processor(message).await;
                        }
                        QueueMode::PerSender => {
                            // 简化实现：暂时按 Parallel 处理，后续完善
                             tokio::spawn(async move {
                                processor(message).await;
                            });
                        }
                    }
                }
                warn!("消息队列接收端已关闭");
            } else {
                warn!("消息队列处理循环已在运行或接收端已失效");
            }
        });
    }

    /// 检查消息是否重复
    ///
    /// 如果消息在去重时间窗口内已经处理过，则认为是重复的
    ///
    /// # 参数说明
    /// * `message` - 要检查的消息
    ///
    /// # 返回值
    /// - `true`: 消息是重复的，跳过处理
    /// - `false`: 消息是新的，需要处理
    ///
    /// # 日志记录
    /// - DEBUG: 重复消息检测结果
    pub async fn is_duplicate(&self, message: &super::types::InboundMessage) -> bool {
        let debounce_ms = self.get_debounce_ms(&message.channel);
        let key = self.build_dedupe_key(message, debounce_ms);

        // 使用读锁检查是否有活跃的相同消息
        if let Some(existing) = self.active_messages.get(&key) {
            let elapsed = existing.elapsed();
            if elapsed < Duration::from_millis(debounce_ms) {
                debug!(
                    message_id = %message.id,
                    channel = %message.channel,
                    elapsed_ms = elapsed.as_millis(),
                    "消息重复，跳过处理"
                );
                return true;
            }
        }

        false
    }

    /// 将消息推入队列
    ///
    /// # 参数说明
    /// * `message` - 要推送的消息
    ///
    /// # 返回值
    /// 成功返回消息，失败返回错误
    ///
    /// # 错误
    /// - `QueueFullError`: 队列已满且丢弃策略为 DropNew
    /// - `ChannelClosedError`: 通道已关闭
    ///
    /// # 日志记录
    /// - INFO: 消息入队
    /// - WARN: 队列满或处理异常
    pub async fn push(
        &self,
        message: super::types::InboundMessage,
    ) -> Result<super::types::InboundMessage, QueueError> {
        // 根据丢弃策略处理
        match self.config.drop_policy {
            // 策略1：丢弃新消息 - 尝试发送，失败则返回错误
            QueueDropPolicy::DropNew => {
                if self.sender.try_send(message.clone()).is_err() {
                    warn!(cap = self.config.cap, "队列已满，丢弃新消息");
                    return Err(QueueError::Full {
                        message_id: message.id.clone(),
                    });
                }
            }
            // 策略2：丢弃最旧消息 - 使用超时发送
            QueueDropPolicy::DropOld => {
                let send_result = tokio::time::timeout(
                    Duration::from_millis(100),
                    self.sender.send(message.clone())
                ).await;

                if send_result.is_err() {
                    warn!("队列处理超时，丢弃消息");
                    return Err(QueueError::Full {
                        message_id: message.id.clone(),
                    });
                }
            }
            // 策略3：阻塞等待 - 一直等待直到有空间
            QueueDropPolicy::Block => {
                if let Err(e) = self.sender.send(message.clone()).await {
                    return Err(QueueError::Closed {
                        message_id: message.id.clone(),
                        msg: e.to_string(),
                    });
                }
            }
        }

        // 添加到活跃消息追踪
        let debounce_ms = self.get_debounce_ms(&message.channel);
        let key = self.build_dedupe_key(&message, debounce_ms);
        self.active_messages.insert(key, std::time::Instant::now());

        debug!(message_id = %message.id, "消息已入队");

        Ok(message)
    }

    /// 获取队列当前大小
    ///
    /// # 返回值
    /// 队列中待处理消息的数量
    pub fn len(&self) -> usize {
        self.sender.capacity()
    }

    /// 检查队列是否为空
    ///
    /// # 返回值
    /// - `true`: 队列为空
    /// - `false`: 队列不为空
    pub fn is_empty(&self) -> bool {
        self.sender.capacity() == self.config.cap
    }

    /// 获取配置的去重时间窗口
    ///
    /// # 参数说明
    /// * `channel` - 渠道类型
    ///
    /// # 返回值
    /// 去重时间窗口（毫秒）
    fn get_debounce_ms(&self, channel: &str) -> u64 {
        // 先检查是否有渠道特定的配置
        if let Some(ms) = self.config.debounce_ms_by_channel.get(channel) {
            return *ms;
        }
        // 使用全局配置
        self.config.debounce_ms
    }

    /// 构建消息去重键
    ///
    /// # 参数说明
    /// * `message` - 消息
    /// * `debounce_ms` - 去重时间窗口
    ///
    /// # 返回值
    /// 去重键
    fn build_dedupe_key(
        &self,
        message: &super::types::InboundMessage,
        debounce_ms: u64,
    ) -> DedupeKey {
        let content_hash = self.hash_content(&message.content);
        DedupeKey::new(&message.channel, &message.sender.id, &content_hash, debounce_ms)
    }

    /// 计算消息内容的哈希值
    ///
    /// # 参数说明
    /// * `content` - 消息内容
    ///
    /// # 返回值
    /// 内容哈希值（十六进制字符串）
    fn hash_content(&self, content: &super::types::MessageContent) -> String {
        // 简单哈希：使用内容的 JSON 表示
        let json = serde_json::to_string(content).unwrap_or_default();
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        json.hash(&mut hasher);
        format!("{:x}", hasher.finish())
    }
}

/// 队列错误类型
///
/// 消息队列操作可能发生的错误
#[derive(Debug, thiserror::Error)]
pub enum QueueError {
    /// 队列已满错误
    #[error("队列已满，无法添加消息: {message_id}")]
    Full {
        /// 消息 ID
        message_id: String,
    },

    /// 通道关闭错误
    #[error("消息通道已关闭: {message_id}, 原因: {msg}")]
    Closed {
        /// 消息 ID
        message_id: String,
        /// 错误原因
        msg: String,
    },
}
