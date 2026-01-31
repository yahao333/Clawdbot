//! 飞书消息发送测试
//!
//! 运行此测试来验证飞书机器人发送消息功能
//!
//! # 使用方法
//! ```bash
//! # 发送文本消息到用户
//! cargo run --example feishu_send_text -- --app-id "xxx" --app-secret "xxx" --to "ou_xxx" --text "Hello!"
//!
//! # 发送 Markdown 卡片消息
//! cargo run --example feishu_send_card -- --app-id "xxx" --app-secret "xxx" --to "ou_xxx" --markdown "**Hello** World"
//!
//! # 发送带按钮的卡片消息
//! cargo run --example feishu_send_card_with_buttons -- --app-id "xxx" --app-secret "xxx" --to "ou_xxx"
//! ```
//!
//! 或者直接运行测试：
//! ```bash
//! cargo test --features feishu feishu_send_text -- --nocapture
//! ```

use clap::{Parser, Subcommand};
use tracing::{info, Level};
use tracing_subscriber;

use clawdbot::channels::feishu::{FeishuCredentials, FeishuMessageSender, MessageReceiver};

/// 飞书消息发送测试参数
#[derive(Parser, Debug)]
#[command(name = "feishu-send")]
#[command(author, version, about, long_about = None)]
struct Args {
    /// 飞书应用 ID
    #[arg(long)]
    app_id: Option<String>,

    /// 飞书应用密钥
    #[arg(long)]
    app_secret: Option<String>,

    /// 接收者 ID（Open ID、User ID 或群聊 ID）
    #[arg(long)]
    to: Option<String>,

    /// 消息文本内容
    #[arg(long)]
    text: Option<String>,

    /// Markdown 格式内容（用于卡片消息）
    #[arg(long)]
    markdown: Option<String>,

    /// 消息类型子命令
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// 发送文本消息
    Text {
        /// 接收者 ID
        #[arg(long)]
        to: String,

        /// 消息内容
        #[arg(long)]
        text: String,
    },

    /// 发送 Markdown 卡片消息
    Card {
        /// 接收者 ID
        #[arg(long)]
        to: String,

        /// Markdown 内容
        #[arg(long)]
        markdown: String,
    },

    /// 发送带按钮的卡片消息
    CardWithButtons {
        /// 接收者 ID
        #[arg(long)]
        to: String,

        /// 标题
        #[arg(long)]
        title: Option<String>,

        /// 文本内容
        #[arg(long)]
        text: Option<String>,
    },

    /// 探测飞书连接
    Probe,

    /// 列出用户
    ListUsers {
        /// 最大数量
        #[arg(long)]
        limit: Option<u32>,
    },

    /// 列出群组
    ListGroups {
        /// 最大数量
        #[arg(long)]
        limit: Option<u32>,
    },
}

/// 列出飞书用户
async fn list_users(credentials: &FeishuCredentials, limit: Option<u32>) -> Result<(), String> {
    use clawdbot::channels::feishu::FeishuClient;

    let client = FeishuClient::new(credentials.clone());
    let token = client.get_access_token().await.map_err(|e| format!("{}", e))?;
    let limit = limit.unwrap_or(50);

    let url = format!(
        "https://open.feishu.cn/open-apis/contact/v3/users?page_size={}",
        limit.min(50)
    );

    let response = client.http_client()
        .get(&url)
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .map_err(|e| format!("网络请求失败: {}", e))?;

    let status = response.status();
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        return Err(format!("HTTP 错误: {} - {}", status, text));
    }

    let text = response.text().await
        .map_err(|e| format!("解析响应失败: {}", e))?;

    info!("用户列表响应: {}", text);
    Ok(())
}

/// 列出飞书群组
async fn list_groups(credentials: &FeishuCredentials, limit: Option<u32>) -> Result<(), String> {
    use clawdbot::channels::feishu::FeishuClient;

    let client = FeishuClient::new(credentials.clone());
    let token = client.get_access_token().await.map_err(|e| format!("{}", e))?;
    let limit = limit.unwrap_or(50);

    let url = format!(
        "https://open.feishu.cn/open-apis/im/v1/chats?page_size={}",
        limit.min(100)
    );

    let response = client.http_client()
        .get(&url)
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .map_err(|e| format!("网络请求失败: {}", e))?;

    let status = response.status();
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        return Err(format!("HTTP 错误: {} - {}", status, text));
    }

    let text = response.text().await
        .map_err(|e| format!("解析响应失败: {}", e))?;

    info!("群组列表响应: {}", text);
    Ok(())
}

/// 简单的飞书探测函数
async fn probe_feishu(credentials: &FeishuCredentials) -> Result<(), String> {
    use clawdbot::channels::feishu::FeishuClient;

    let client = FeishuClient::new(credentials.clone());

    // 尝试获取访问令牌
    match client.get_access_token().await {
        Ok(token) => {
            info!("获取访问令牌成功: {}", &token[..20.min(token.len())]);
        }
        Err(e) => {
            return Err(format!("获取访问令牌失败: {}", e));
        }
    }

    // 尝试获取机器人信息
    let url = "https://open.feishu.cn/open-apis/bot/v3/info";
    let token = client.get_access_token().await.map_err(|e| format!("{}", e))?;
    let response = client.http_client()
        .get(url)
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .map_err(|e| format!("网络请求失败: {}", e))?;

    let status = response.status();
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        return Err(format!("HTTP 错误: {} - {}", status, text));
    }

    let text = response.text().await
        .map_err(|e| format!("解析响应失败: {}", e))?;

    info!("探测响应: {}", text);
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), String> {
    // 初始化日志
    tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .init();

    // 加载环境变量
    dotenv::dotenv().ok();

    let args = Args::parse();

    // 获取凭证
    let app_id = args.app_id
        .or_else(|| std::env::var("FEISHU_APP_ID").ok())
        .expect("请提供 --app-id 或设置 FEISHU_APP_ID 环境变量");

    let app_secret = args.app_secret
        .or_else(|| std::env::var("FEISHU_APP_SECRET").ok())
        .expect("请提供 --app-secret 或设置 FEISHU_APP_SECRET 环境变量");

    let credentials = FeishuCredentials {
        app_id,
        app_secret,
        verification_token: None,
        encrypt_key: None,
    };

    info!("飞书凭证已加载: app_id = {}", credentials.app_id);

    // 探测连接
    info!("探测飞书连接...");
    if let Err(e) = probe_feishu(&credentials).await {
        info!("探测失败: {}", e);
    }

    // 创建发送器
    let sender = FeishuMessageSender::new(credentials.clone());

    // 处理子命令
    match &args.command {
        Some(Commands::Text { to, text }) => {
            info!("发送文本消息到: {}", to);
            let receiver = MessageReceiver::OpenId(to.clone());
            match sender.send_text(&receiver, text).await {
                Ok(response) => {
                    info!("消息发送成功! message_id = {}", response.message_id);
                }
                Err(e) => {
                    info!("消息发送失败: {}", e);
                    return Err(e.to_string());
                }
            }
        }

        Some(Commands::Card { to, markdown }) => {
            info!("发送 Markdown 卡片到: {}", to);
            let receiver = MessageReceiver::OpenId(to.clone());
            match sender.send_markdown_card(&receiver, markdown).await {
                Ok(response) => {
                    info!("卡片发送成功! message_id = {}", response.message_id);
                }
                Err(e) => {
                    info!("卡片发送失败: {}", e);
                    return Err(e.to_string());
                }
            }
        }

        Some(Commands::CardWithButtons { to, title, text }) => {
            info!("发送带按钮的卡片到: {}", to);
            let receiver = MessageReceiver::OpenId(to.clone());

            let title = title.as_deref().unwrap_or("测试标题");
            let text = text.as_deref().unwrap_or("这是卡片消息的正文内容。");

            use clawdbot::channels::feishu::CardButton;
            let buttons = vec![
                CardButton::new("确定", "btn_confirm").primary(),
                CardButton::new("取消", "btn_cancel").danger(),
            ];

            match sender.send_card_with_buttons(&receiver, title, text, &buttons).await {
                Ok(response) => {
                    info!("按钮卡片发送成功! message_id = {}", response.message_id);
                }
                Err(e) => {
                    info!("按钮卡片发送失败: {}", e);
                    return Err(e.to_string());
                }
            }
        }

        Some(Commands::Probe) => {
            info!("执行飞书探测...");
        }

        Some(Commands::ListUsers { limit }) => {
            info!("列出用户...");
            if let Err(e) = list_users(&credentials, *limit).await {
                info!("列出用户失败: {}", e);
            }
        }

        Some(Commands::ListGroups { limit }) => {
            info!("列出群组...");
            if let Err(e) = list_groups(&credentials, *limit).await {
                info!("列出群组失败: {}", e);
            }
        }

        None => {
            // 使用命令行参数
            if let (Some(to), Some(text)) = (&args.to, &args.text) {
                info!("发送文本消息到: {}", to);
                let receiver = MessageReceiver::OpenId(to.clone());
                match sender.send_text(&receiver, text).await {
                    Ok(response) => {
                        info!("消息发送成功! message_id = {}", response.message_id);
                    }
                    Err(e) => {
                        info!("消息发送失败: {}", e);
                        return Err(e.to_string());
                    }
                }
            } else if let (Some(to), Some(markdown)) = (&args.to, &args.markdown) {
                info!("发送 Markdown 卡片到: {}", to);
                let receiver = MessageReceiver::OpenId(to.clone());
                match sender.send_markdown_card(&receiver, markdown).await {
                    Ok(response) => {
                        info!("卡片发送成功! message_id = {}", response.message_id);
                    }
                    Err(e) => {
                        info!("卡片发送失败: {}", e);
                        return Err(e.to_string());
                    }
                }
            } else {
                info!("请提供 --to 和 --text 或 --markdown 参数");
                info!("示例: feishu-send --to \"ou_xxx\" --text \"Hello!\"");
                info!("或使用子命令: feishu-send text --to \"ou_xxx\" --text \"Hello!\"");
                info!("或: feishu-send card --to \"ou_xxx\" --markdown \"**Hello**\"");
            }
        }
    }

    Ok(())
}
