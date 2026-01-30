//! 消息路由模块
//!
//! 本模块负责将消息路由到对应的 Agent。

pub mod engine;
// 暂时移除未实现的模块
// pub mod resolver;
// pub mod rules;
// pub mod session_key;

// 重新导出常用类型
pub use engine::{Router, RouteMatch, RouteRule, DefaultRouter};
