//! 渠道适配器模块
//!
//! 本模块定义了渠道适配器的统一接口，并实现了各种即时通讯平台的适配器。

pub mod traits;
pub mod feishu;
// 暂时移除未实现的渠道
// pub mod telegram;
// pub mod discord;
// pub mod slack;
// pub mod common;
