//! 飞书群组策略模块
//!
//! 实现群组级别的消息处理策略控制。
//!
//! # 功能
//! 1. 黑名单/白名单控制
//! 2. @提及要求配置
//! 3. 群组策略继承
//!
//! # 策略类型
//! - `Open`: 完全开放，机器人响应所有消息
//! - `Allowlist`: 仅白名单群组可用
//! - `Private`: 仅私聊可用
//!
//! # 使用示例
//! ```rust
//! let strategy = GroupStrategy::new();
//! strategy.set_chat_policy("oc_xxx", Policy::Allowlist).await?;
//! ```

use crate::channels::feishu::FeishuClient;
use crate::infra::error::{Error, Result};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{debug, info, warn};

/// 群组策略类型
///
/// 控制机器人在群组中的行为模式
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum GroupPolicy {
    /// 开放策略 - 响应所有消息（包括私聊和群组）
    Open,
    /// 白名单策略 - 仅响应白名单群组的消息
    Allowlist,
    /// 私聊策略 - 仅响应私聊，不响应群组
    Private,
    /// 黑名单策略 - 响应所有消息，除黑名单群组外
    Blacklist,
}

/// 群组策略配置
///
/// 存储特定群组的策略设置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatPolicy {
    /// 群组 ID（chat_id）
    pub chat_id: String,
    /// 策略类型
    pub policy: GroupPolicy,
    /// 是否需要 @ 提及才响应（群组模式）
    pub require_mention: bool,
    /// 管理员用户 ID 列表（可覆盖策略）
    pub admin_ids: Vec<String>,
    /// 机器人名称（用于检测 @ 提及）
    pub bot_name: Option<String>,
    /// 是否启用
    pub enabled: bool,
    /// 备注
    pub remark: Option<String>,
}

/// 群组策略管理器
///
/// 管理和应用群组级别的消息处理策略
///
/// # 策略优先级
/// 1. 管理员用户不受策略限制
/// 2. 白名单群组优先于黑名单
/// 3. 默认策略用于未知群组
///
/// # 缓存策略
/// - 使用 DashMap 进行内存缓存
/// - 支持动态更新策略
/// - 可选持久化到数据库
#[derive(Clone)]
pub struct GroupStrategyManager {
    /// 策略缓存
    policies: Arc<DashMap<String, ChatPolicy>>,
    /// 默认策略
    default_policy: GroupPolicy,
    /// 是否默认需要 @ 提及
    default_require_mention: bool,
    /// 飞书客户端（用于获取群组信息）
    feishu_client: Option<FeishuClient>,
    /// 白名单群组列表
    allowlist: Arc<DashMap<String, bool>>,
    /// 黑名单群组列表
    blacklist: Arc<DashMap<String, bool>>,
    /// 管理员用户 ID 集合
    admin_ids: Arc<DashMap<String, bool>>,
}

impl Default for GroupStrategyManager {
    fn default() -> Self {
        Self::new()
    }
}

impl GroupStrategyManager {
    /// 创建策略管理器
    pub fn new() -> Self {
        Self {
            policies: Arc::new(DashMap::new()),
            default_policy: GroupPolicy::Open,
            default_require_mention: false,
            feishu_client: None,
            allowlist: Arc::new(DashMap::new()),
            blacklist: Arc::new(DashMap::new()),
            admin_ids: Arc::new(DashMap::new()),
        }
    }

    /// 创建带飞书客户端的策略管理器
    pub fn with_client(feishu_client: FeishuClient) -> Self {
        let mut manager = Self::new();
        manager.feishu_client = Some(feishu_client);
        manager
    }

    /// 设置默认策略
    ///
    /// # 参数说明
    /// * `policy` - 默认策略类型
    /// * `require_mention` - 默认是否需要 @ 提及
    pub fn set_default(&mut self, policy: &GroupPolicy, require_mention: bool) {
        self.default_policy = policy.clone();
        self.default_require_mention = require_mention;
        debug!(policy = ?policy, require_mention, "默认策略已更新");
    }

    /// 添加到白名单
    pub fn add_to_allowlist(&self, chat_id: &str) {
        self.allowlist.insert(chat_id.to_string(), true);
        debug!(chat_id = %chat_id, "添加到白名单");
    }

    /// 从白名单移除
    pub fn remove_from_allowlist(&self, chat_id: &str) {
        self.allowlist.remove(chat_id);
        debug!(chat_id = %chat_id, "从白名单移除");
    }

    /// 添加到黑名单
    pub fn add_to_blacklist(&self, chat_id: &str) {
        self.blacklist.insert(chat_id.to_string(), true);
        debug!(chat_id = %chat_id, "添加到黑名单");
    }

    /// 从黑名单移除
    pub fn remove_from_blacklist(&self, chat_id: &str) {
        self.blacklist.remove(chat_id);
        debug!(chat_id = %chat_id, "从黑名单移除");
    }

    /// 添加管理员
    pub fn add_admin(&self, user_id: &str) {
        self.admin_ids.insert(user_id.to_string(), true);
        debug!(user_id = %user_id, "添加管理员");
    }

    /// 移除管理员
    pub fn remove_admin(&self, user_id: &str) {
        self.admin_ids.remove(user_id);
        debug!(user_id = %user_id, "移除管理员");
    }

    /// 设置群组策略
    pub fn set_chat_policy(&self, chat_id: &str, policy: &GroupPolicy, require_mention: bool) {
        let chat_policy = ChatPolicy {
            chat_id: chat_id.to_string(),
            policy: policy.clone(),
            require_mention,
            admin_ids: Vec::new(),
            bot_name: None,
            enabled: true,
            remark: None,
        };
        self.policies.insert(chat_id.to_string(), chat_policy);
        debug!(chat_id = %chat_id, policy = ?policy, require_mention, "群组策略已设置");
    }

    /// 获取群组策略
    pub fn get_chat_policy(&self, chat_id: &str) -> ChatPolicy {
        // 1. 先检查自定义策略
        if let Some(policy) = self.policies.get(chat_id) {
            return policy.clone();
        }

        // 2. 返回默认策略
        ChatPolicy {
            chat_id: chat_id.to_string(),
            policy: self.default_policy.clone(),
            require_mention: self.default_require_mention,
            admin_ids: Vec::new(),
            bot_name: None,
            enabled: true,
            remark: None,
        }
    }

    /// 检查是否应该处理消息
    ///
    /// # 参数说明
    /// * `chat_id` - 群组 ID
    /// * `chat_type` - 聊天类型（p2p 或 group）
    /// * `sender_id` - 发送者 ID
    /// * `content` - 消息内容（用于检测 @ 提及）
    ///
    /// # 返回值
    /// 是否应该处理此消息
    pub async fn should_handle(
        &self,
        chat_id: &str,
        chat_type: &str,
        sender_id: &str,
        content: &str,
    ) -> bool {
        // 1. 管理员不受策略限制
        if self.admin_ids.contains_key(sender_id) {
            debug!(sender_id = %sender_id, "管理员跳过策略检查");
            return true;
        }

        // 2. 私聊直接处理（除非设置了 Private 策略）
        if chat_type == "p2p" {
            match self.default_policy {
                GroupPolicy::Private => {
                    debug!(chat_id = %chat_id, "私聊模式，不处理群组消息");
                    return false;
                }
                _ => {
                    return true;
                }
            }
        }

        // 3. 群组消息处理
        // 检查黑名单
        if self.blacklist.contains_key(chat_id) {
            warn!(chat_id = %chat_id, "群组在黑名单中");
            return false;
        }

        // 检查白名单
        if self.allowlist.contains_key(chat_id) {
            debug!(chat_id = %chat_id, "群组在白名单中");
            return true;
        }

        // 根据策略决定
        match self.default_policy {
            GroupPolicy::Allowlist => {
                debug!(chat_id = %chat_id, "白名单模式，群组不在白名单中");
                false
            }
            GroupPolicy::Private => {
                debug!(chat_id = %chat_id, "私聊模式，不处理群组消息");
                false
            }
            GroupPolicy::Blacklist => {
                // 黑名单模式已在上面处理
                true
            }
            GroupPolicy::Open => true,
        }
    }

    /// 检查是否需要 @ 提及
    ///
    /// # 参数说明
    /// * `chat_id` - 群组 ID
    /// * `content` - 消息内容
    ///
    /// # 返回值
    /// 是否需要 @ 提及才响应
    pub fn require_mention(&self, chat_id: &str, content: &str) -> bool {
        // 1. 检查群组自定义策略
        if let Some(policy) = self.policies.get(chat_id) {
            if !policy.require_mention {
                return false;
            }
        } else if !self.default_require_mention {
            return false;
        }

        // 2. 检查消息内容是否包含 @ 提及
        // 飞书 @ 提及格式: <at user_id="xxx">@用户名</at>
        if content.contains("<at user_id=") {
            return false; // 包含 @ 提及，不需要额外检查
        }

        // 检查纯文本 @ 格式
        if content.contains("@") {
            return false;
        }

        true
    }

    /// 批量加载白名单
    pub fn load_allowlist(&self, chat_ids: &[String]) {
        for chat_id in chat_ids {
            self.allowlist.insert(chat_id.clone(), true);
        }
        info!(count = chat_ids.len(), "白名单已加载");
    }

    /// 批量加载黑名单
    pub fn load_blacklist(&self, chat_ids: &[String]) {
        for chat_id in chat_ids {
            self.blacklist.insert(chat_id.clone(), true);
        }
        info!(count = chat_ids.len(), "黑名单已加载");
    }

    /// 批量加载管理员
    pub fn load_admins(&self, user_ids: &[String]) {
        for user_id in user_ids {
            self.admin_ids.insert(user_id.clone(), true);
        }
        info!(count = user_ids.len(), "管理员列表已加载");
    }

    /// 获取统计信息
    pub fn stats(&self) -> StrategyStats {
        StrategyStats {
            policy_count: self.policies.len(),
            allowlist_count: self.allowlist.len(),
            blacklist_count: self.blacklist.len(),
            admin_count: self.admin_ids.len(),
            default_policy: self.default_policy.clone(),
        }
    }
}

/// 策略统计信息
#[derive(Debug, Serialize)]
pub struct StrategyStats {
    /// 自定义策略数量
    pub policy_count: usize,
    /// 白名单数量
    pub allowlist_count: usize,
    /// 黑名单数量
    pub blacklist_count: usize,
    /// 管理员数量
    pub admin_count: usize,
    /// 默认策略
    pub default_policy: GroupPolicy,
}

/// 策略决策结果
#[derive(Debug, Clone)]
pub struct StrategyDecision {
    /// 是否应该处理
    pub should_handle: bool,
    /// 原因说明
    pub reason: String,
    /// 匹配到的策略
    pub matched_policy: Option<GroupPolicy>,
    /// 是否需要 @ 提及
    pub require_mention: bool,
}

impl StrategyDecision {
    /// 创建允许决策
    pub fn allowed(policy: GroupPolicy) -> Self {
        Self {
            should_handle: true,
            reason: "策略允许".to_string(),
            matched_policy: Some(policy),
            require_mention: false,
        }
    }

    /// 创建拒绝决策
    pub fn denied(reason: &str) -> Self {
        Self {
            should_handle: false,
            reason: reason.to_string(),
            matched_policy: None,
            require_mention: false,
        }
    }
}
