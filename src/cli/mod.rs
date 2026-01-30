//! CLI 命令行入口模块
//!
//! 本模块负责：
//! 1. 解析命令行参数
//! 2. 显示启动横幅
//! 3. 构建 CLI 程序
//!
//! # 使用示例
//! ```bash
//! clawdbot --config config.toml --verbose
//! ```

pub mod program;
pub mod argv;
pub mod banner;
