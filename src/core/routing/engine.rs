//! 路由引擎模块
//!
//! 实现消息路由的核心逻辑。

use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::debug;

use crate::core::message::types::InboundMessage;
use crate::infra::error::Result;

/// 路由匹配结果
///
/// 记录路由匹配的结果信息
#[derive(Debug, Clone)]
pub struct RouteMatch {
    /// 匹配的 Agent ID
    pub agent_id: String,
    /// 匹配置信度（0.0 - 1.0）
    pub confidence: f64,
    /// 匹配的规则列表
    pub rules_matched: Vec<String>,
}

/// 路由规则
///
/// 定义消息路由的规则
#[derive(Debug, Clone)]
pub struct RouteRule {
    /// 规则 ID
    pub id: String,
    /// 规则名称
    pub name: String,
    /// 规则优先级（数值越大优先级越高）
    pub priority: u32,
    /// 是否启用
    pub enabled: bool,
    /// 规则条件
    pub condition: RouteCondition,
    /// 匹配的 Agent ID
    pub agent_id: String,
}

/// 路由条件
///
/// 定义路由规则的条件匹配方式
#[derive(Debug, Clone)]
pub enum RouteCondition {
    /// 渠道匹配
    Channel {
        /// 渠道列表
        channels: Vec<String>,
    },
    /// 发送者匹配
    Sender {
        /// 发送者 ID 列表
        sender_ids: Vec<String>,
    },
    /// 发送者用户名匹配
    SenderUsername {
        /// 用户名匹配模式
        pattern: String,
        /// 是否使用正则表达式
        regex: bool,
    },
    /// 内容匹配
    Content {
        /// 内容匹配模式
        pattern: String,
        /// 是否使用正则表达式
        regex: bool,
    },
    /// 组合条件（AND）
    And(Vec<RouteCondition>),
    /// 组合条件（OR）
    Or(Vec<RouteCondition>),
    /// 否定条件
    Not(Box<RouteCondition>),
}

/// 路由器 Trait
///
/// 定义路由器的接口
#[async_trait::async_trait]
pub trait Router: Send + Sync {
    /// 路由消息
    ///
    /// # 参数说明
    /// * `message` - 入站消息
    ///
    /// # 返回值
    /// 路由匹配结果
    async fn route(&self, message: &InboundMessage) -> Result<RouteMatch>;

    /// 添加路由规则
    ///
    /// # 参数说明
    /// * `rule` - 路由规则
    async fn add_rule(&self, rule: RouteRule) -> Result<()>;

    /// 移除路由规则
    ///
    /// # 参数说明
    /// * `rule_id` - 规则 ID
    async fn remove_rule(&self, rule_id: &str) -> Result<()>;

    /// 列出所有规则
    async fn list_rules(&self) -> Vec<RouteRule>;
}

/// 默认路由器实现
///
/// 标准路由器实现，支持规则匹配和回退
#[derive(Clone, Debug)]
pub struct DefaultRouter {
    /// 路由规则列表
    rules: Arc<RwLock<Vec<RouteRule>>>,
    /// 默认回退 Agent ID
    fallback_agent: String,
}

impl DefaultRouter {
    /// 创建路由器
    ///
    /// # 参数说明
    /// * `fallback_agent` - 默认回退 Agent ID
    ///
    /// # 返回值
    /// 创建的路由器
    pub fn new(fallback_agent: &str) -> Self {
        Self {
            rules: Arc::new(RwLock::new(Vec::new())),
            fallback_agent: fallback_agent.to_string(),
        }
    }
}

#[async_trait::async_trait]
impl Router for DefaultRouter {
    async fn route(&self, message: &InboundMessage) -> Result<RouteMatch> {
        let rules = self.rules.read().await;
        let mut matches: Vec<RouteMatch> = Vec::new();

        // 遍历所有启用的规则
        for rule in rules.iter().filter(|r| r.enabled) {
            if self.matches_condition(&rule.condition, message) {
                debug!(rule_id = %rule.id, rule_name = %rule.name, "规则匹配成功");
                matches.push(RouteMatch {
                    agent_id: rule.agent_id.clone(),
                    confidence: 1.0,
                    rules_matched: vec![rule.id.clone()],
                });
            }
        }

        // 按优先级排序
        matches.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap_or(std::cmp::Ordering::Equal));

        // 返回最佳匹配或回退
        if let Some(best) = matches.into_iter().max_by(|a, b| {
            a.confidence.partial_cmp(&b.confidence).unwrap_or(std::cmp::Ordering::Equal)
        }) {
            Ok(best)
        } else {
            // 没有匹配规则，使用回退 Agent
            Ok(RouteMatch {
                agent_id: self.fallback_agent.clone(),
                confidence: 0.5,
                rules_matched: vec!["fallback".to_string()],
            })
        }
    }

    async fn add_rule(&self, rule: RouteRule) -> Result<()> {
        let mut rules = self.rules.write().await;
        rules.push(rule);
        // 按优先级降序排序
        rules.sort_by(|a, b| b.priority.cmp(&a.priority));
        Ok(())
    }

    async fn remove_rule(&self, rule_id: &str) -> Result<()> {
        let mut rules = self.rules.write().await;
        rules.retain(|r| r.id != rule_id);
        Ok(())
    }

    async fn list_rules(&self) -> Vec<RouteRule> {
        self.rules.read().await.clone()
    }
}

impl DefaultRouter {
    /// 检查消息是否匹配条件
    fn matches_condition(&self, condition: &RouteCondition, message: &InboundMessage) -> bool {
        match condition {
            RouteCondition::Channel { channels } => {
                channels.iter().any(|c| c == &message.channel)
            }
            RouteCondition::Sender { sender_ids } => {
                sender_ids.iter().any(|id| id == &message.sender.id)
            }
            RouteCondition::SenderUsername { pattern, regex: _ } => {
                message.sender.username.as_ref().map(|u| u.contains(pattern)).unwrap_or(false)
            }
            RouteCondition::Content { pattern, regex: _ } => {
                message.content.text.as_ref().map(|t| t.contains(pattern)).unwrap_or(false)
            }
            RouteCondition::And(conditions) => {
                conditions.iter().all(|c| self.matches_condition(c, message))
            }
            RouteCondition::Or(conditions) => {
                conditions.iter().any(|c| self.matches_condition(c, message))
            }
            RouteCondition::Not(condition) => {
                !self.matches_condition(condition, message)
            }
        }
    }
}
