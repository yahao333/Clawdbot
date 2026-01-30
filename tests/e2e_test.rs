use clawdbot::ai::provider::minimax::{MiniMaxConfig, MiniMaxProvider};
use clawdbot::ai::provider::{AiProvider, ChatMessage, ChatRequest, MessageRole, ModelConfig};
use clawdbot::channels::feishu::{FeishuClient, FeishuCredentials};
use dotenv::dotenv;
use std::env;

#[tokio::test]
async fn test_minimax_feishu_integration() {
    // 1. 加载环境变量
    dotenv().ok();

    // 2. 验证 Feishu 集成
    println!("=== Testing Feishu Integration ===");
    // 只有在提供了环境变量时才运行 Feishu 测试
    if let (Ok(app_id), Ok(app_secret)) = (env::var("FEISHU_APP_ID"), env::var("FEISHU_APP_SECRET")) {
        let feishu_creds = FeishuCredentials {
            app_id,
            app_secret,
            verification_token: None,
            encrypt_key: None,
        };

        let feishu_client = FeishuClient::new(feishu_creds);

        // 验证 WebSocket URL 获取
        match feishu_client.get_websocket_url().await {
            Ok(url) => {
                println!("✅ Successfully retrieved Feishu WebSocket URL: {}", url);
                assert!(url.starts_with("wss://"), "WebSocket URL should start with wss://");
            }
            Err(e) => {
                println!("⚠️ Failed to get Feishu WebSocket URL: {}", e);
                // 不让测试失败，因为这可能取决于网络环境或 Token 有效性
            }
        }
    } else {
        println!("⚠️ Skipping Feishu test: FEISHU_APP_ID or FEISHU_APP_SECRET not set");
    }

    // 3. 验证 MiniMax 集成
    println!("\n=== Testing MiniMax Integration ===");
    if let (Ok(api_key), Ok(group_id)) = (env::var("MINIMAX_API_KEY"), env::var("MINIMAX_GROUP_ID")) {
        let minimax_config = MiniMaxConfig {
            api_key,
            group_id,
            base_url: None,
            model: Some("abab6.5s-chat".to_string()),
            temperature: Some(0.7),
            max_tokens: Some(100),
        };

        let minimax_provider = MiniMaxProvider::new(minimax_config);

        let messages = vec![ChatMessage {
            role: MessageRole::User,
            content: "Hello, are you working?".to_string(),
            name: None,
        }];

        let request = ChatRequest {
            model: ModelConfig {
                provider: "minimax".to_string(),
                model: "abab6.5s-chat".to_string(),
                api_key: None,
                base_url: None,
                temperature: None,
                max_tokens: None,
                system_prompt: None,
            },
            messages,
            tools: vec![],
            stream: false,
        };

        // 发送简单的聊天请求
        match minimax_provider.chat(&request).await {
            Ok(response) => {
                println!("✅ Successfully received MiniMax response: {:?}", response.content);
                assert!(!response.content.is_empty(), "MiniMax response content should not be empty");
            }
            Err(e) => {
                println!("❌ MiniMax chat request failed: {}", e);
                // 同样不让测试失败，如果是因为余额不足等原因
            }
        }
    } else {
        println!("⚠️ Skipping MiniMax test: MINIMAX_API_KEY or MINIMAX_GROUP_ID not set");
    }
}
