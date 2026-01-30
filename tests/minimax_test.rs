//! MiniMax API 简单测试

use clawdbot::ai::provider::{AiProvider, ChatMessage, ChatRequest, MessageRole, ModelConfig};
use clawdbot::ai::provider::minimax::{MiniMaxConfig, MiniMaxProvider};

#[tokio::test]
async fn test_minimax_chat() {
    // 加载 .env 文件
    dotenv::dotenv().ok();

    println!("=== MiniMax API 测试 ===\n");

    // 从环境变量加载配置
    let api_key = std::env::var("MINIMAX_API_KEY").expect("请设置 MINIMAX_API_KEY");
    let group_id = std::env::var("MINIMAX_GROUP_ID").expect("请设置 MINIMAX_GROUP_ID");

    println!("API Key: {}...", &api_key[..20]);
    println!("Group ID: {}\n", group_id);

    // 创建 MiniMax 配置
    let config = MiniMaxConfig {
        api_key,
        group_id,
        model: Some("abab6.5s-chat".to_string()),
        temperature: Some(0.7),
        max_tokens: Some(100),
        base_url: None,
    };

    // 创建 Provider
    let provider = MiniMaxProvider::new(config);
    println!("Provider 名称: {}\n", provider.name());

    // 创建测试请求
    let request = ChatRequest {
        model: ModelConfig {
            provider: "minimax".to_string(),
            model: "abab6.5s-chat".to_string(),
            api_key: None,
            base_url: None,
            temperature: Some(0.7),
            max_tokens: Some(100),
            system_prompt: None,
        },
        messages: vec![
            ChatMessage {
                role: MessageRole::User,
                content: "你好，请介绍一下你自己".to_string(),
                name: None,
            },
        ],
        tools: vec![],
        stream: false,
    };

    println!("发送请求到 MiniMax API...\n");

    match provider.chat(&request).await {
        Ok(response) => {
            println!("=== 响应成功 ===");
            println!("响应 ID: {}", response.id);
            println!("响应内容:\n{}", response.content);
            println!("\nToken 使用:");
            println!("  - Prompt Tokens: {}", response.usage.prompt_tokens);
            println!("  - Completion Tokens: {}", response.usage.completion_tokens);
            println!("  - Total Tokens: {}", response.usage.total_tokens);
            println!("\n完成: {}", response.done);

            // 验证响应
            assert!(!response.id.is_empty(), "响应 ID 不应为空");
            assert!(!response.content.is_empty(), "响应内容不应为空");
        }
        Err(e) => {
            println!("=== 响应失败 ===");
            println!("错误: {}", e);
            panic!("MiniMax API 调用失败: {}", e);
        }
    }

    println!("\n=== 测试完成 ===");
}
