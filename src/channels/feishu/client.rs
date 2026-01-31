//! 飞书客户端模块
//!
//! 封装飞书 API 的 HTTP 客户端。
//!
//! # 功能
//! 1. 获取访问令牌（App Access Token）
//! 2. 发送消息
//! 3. 上传文件
//!
//! # 认证流程
//! ```
//! 1. 使用 app_id 和 app_secret 获取 app_access_token
//! 2. 在 HTTP 请求头中添加 Authorization: Bearer {token}
//! ```

use reqwest;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{debug, error, info, warn};

use crate::infra::error::{Error, Result};

/// 飞书凭证配置
///
/// 存储飞书应用的认证信息
///
/// # 敏感信息
/// - `app_secret` 是应用密钥，必须保密
/// - 不要将凭据硬编码在代码中
#[derive(Debug, Clone)]
pub struct FeishuCredentials {
    /// 应用 ID
    pub app_id: String,
    /// 应用密钥
    pub app_secret: String,
    /// 验证令牌（用于事件订阅）
    pub verification_token: Option<String>,
    /// 加密密钥（用于加密事件）
    pub encrypt_key: Option<String>,
}

/// 飞书客户端
///
/// 用于调用飞书 API 的 HTTP 客户端
///
/// # 字段说明
/// * `credentials` - 认证凭证
/// * `http_client` - HTTP 客户端
/// * `base_url` - API 基础 URL
/// * `access_token` - 访问令牌（缓存）
/// * `token_expires_at` - 令牌过期时间
///
/// # 使用示例
/// ```rust
/// let client = FeishuClient::new(credentials);
/// let messages = client.send_text("user_id", "Hello!").await?;
/// ```
#[derive(Clone)]
pub struct FeishuClient {
    /// 认证凭证（仅存储 ID 用于调试）
    credentials: Arc<FeishuCredentials>,
    /// HTTP 客户端
    http_client: reqwest::Client,
    /// API 基础 URL
    base_url: String,
    /// 访问令牌（缓存）
    access_token: Arc<tokio::sync::RwLock<Option<String>>>,
    /// 令牌过期时间
    token_expires_at: Arc<tokio::sync::RwLock<Option<i64>>>,
}

impl std::fmt::Debug for FeishuClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FeishuClient")
            .field("base_url", &self.base_url)
            .finish()
    }
}

impl FeishuClient {
    /// 创建飞书客户端
    ///
    /// # 参数说明
    /// * `credentials` - 认证凭证
    ///
    /// # 返回值
    /// 创建的客户端
    pub fn new(credentials: FeishuCredentials) -> Self {
        let http_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("创建 HTTP 客户端失败");

        Self {
            credentials: Arc::new(credentials),
            http_client,
            base_url: "https://open.feishu.cn/open-apis".to_string(),
            access_token: Arc::new(tokio::sync::RwLock::new(None)),
            token_expires_at: Arc::new(tokio::sync::RwLock::new(None)),
        }
    }

    /// 获取访问令牌
    ///
    /// 飞书使用 app_access_token 进行 API 认证
    ///
    /// # 返回值
    /// 访问令牌
    ///
    /// # 日志记录
    /// - DEBUG: 获取令牌过程
    /// - INFO: 获取成功
    /// - ERROR: 获取失败
    pub async fn get_access_token(&self) -> Result<String> {
        // 检查缓存的令牌是否有效
        {
            let token = self.access_token.read().await;
            let expires_at = *self.token_expires_at.read().await;
            if let (Some(t), Some(exp)) = (&*token, expires_at) {
                // 如果令牌还有 5 分钟以上有效期，直接使用
                if exp - chrono::Utc::now().timestamp() > 300 {
                    debug!("使用缓存的访问令牌");
                    return Ok(t.clone());
                }
            }
        }

        debug!("缓存令牌无效或过期，重新获取");

        // 构建请求
        let url = format!("{}/auth/v3/app_access_token/internal", self.base_url);

        #[derive(Serialize)]
        struct Request {
            app_id: String,
            app_secret: String,
        }

        let request_body = Request {
            app_id: self.credentials.app_id.clone(),
            app_secret: self.credentials.app_secret.clone(),
        };

        debug!(app_id = %request_body.app_id, "请求访问令牌");

        // 发送请求
        let response = self.http_client
            .post(&url)
            .json(&request_body)
            .send()
            .await
            .map_err(|e| Error::Network(format!("请求访问令牌失败: {}", e)))?;

        // 解析响应
        #[derive(Deserialize)]
        struct Response {
            code: i64,
            msg: String,
            app_access_token: Option<String>,
            expire: Option<i64>,
        }

        let response_body: Response = response
            .json()
            .await
            .map_err(|e| Error::Network(format!("解析访问令牌响应失败: {}", e)))?;

        // 检查响应状态
        if response_body.code != 0 {
            error!(code = response_body.code, msg = response_body.msg, "获取访问令牌失败");
            return Err(Error::Auth(format!("获取访问令牌失败: {}", response_body.msg)));
        }

        // 缓存令牌
        if let Some(token) = response_body.app_access_token {
            let mut token_guard = self.access_token.write().await;
            *token_guard = Some(token.clone());

            if let Some(expire) = response_body.expire {
                let mut expires_guard = self.token_expires_at.write().await;
                *expires_guard = Some(expire);
            }

            info!("获取访问令牌成功");
            Ok(token)
        } else {
            Err(Error::Auth("响应中未包含访问令牌".to_string()))
        }
    }

    /// 获取 HTTP 客户端（用于自定义请求）
    pub fn http_client(&self) -> &reqwest::Client {
        &self.http_client
    }

    /// 发送 API 请求
    ///
    /// 通用方法，用于发送经过认证的 API 请求
    ///
    /// # 参数说明
    /// * `method` - HTTP 方法
    /// * `path` - API 路径
    /// * `body` - 请求体（可选）
    ///
    /// # 返回值
    /// 响应 JSON
    pub async fn request<T: for<'de> Deserialize<'de>>(
        &self,
        method: &str,
        path: &str,
        body: Option<impl Serialize>,
    ) -> Result<T> {
        let token = self.get_access_token().await?;
        let url = format!("{}{}", self.base_url, path);

        let mut request = self.http_client
            .request(
                match method.to_uppercase().as_str() {
                    "GET" => reqwest::Method::GET,
                    "POST" => reqwest::Method::POST,
                    "PUT" => reqwest::Method::PUT,
                    "DELETE" => reqwest::Method::DELETE,
                    _ => reqwest::Method::GET,
                },
                &url,
            )
            .header("Authorization", format!("Bearer {}", token))
            .header("Content-Type", "application/json");

        if let Some(body) = body {
            request = request.json(&body);
        }

        let response = request
            .send()
            .await
            .map_err(|e| Error::Network(format!("API 请求失败: {}", e)))?;

        #[derive(Deserialize)]
        struct ApiResponse<T> {
            code: i64,
            msg: String,
            data: Option<T>,
        }

        let response_body: ApiResponse<T> = response
            .json()
            .await
            .map_err(|e| Error::Network(format!("解析 API 响应失败: {}", e)))?;

        if response_body.code != 0 {
            error!(code = response_body.code, msg = response_body.msg, "API 请求失败");
            return Err(Error::Channel(format!("飞书 API 错误: {}", response_body.msg)));
        }

        response_body.data
            .ok_or_else(|| Error::Channel("响应中未包含数据".to_string()))
    }

    /// 获取消息列表
    ///
    /// 用于长链接轮询获取消息
    ///
    /// # 参数说明
    /// * `container_id_type` - 容器类型（目前仅支持 "chat"）
    /// * `container_id` - 容器 ID（即 chat_id）
    /// * `page_size` - 每页消息数量
    ///
    /// # 返回值
    /// 消息列表响应
    pub async fn get_messages(&self, container_id_type: &str, container_id: &str, page_size: u32) -> Result<super::handlers::MessageListResponse> {
        let token = self.get_access_token().await?;
        let url = format!("{}/im/v1/messages", self.base_url);

        let response = self.http_client
            .get(&url)
            .header("Authorization", format!("Bearer {}", token))
            .query(&[
                ("container_id_type", container_id_type),
                ("container_id", container_id),
                ("page_size", &page_size.to_string()),
                ("sort", "-create_time"),
            ])
            .send()
            .await
            .map_err(|e| Error::Network(format!("获取消息失败: {}", e)))?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(Error::Channel(format!("获取消息失败: {}", error_text)));
        }

        let response_text = response.text().await
            .map_err(|e| Error::Network(format!("解析消息响应失败: {}", e)))?;

        let response: super::handlers::MessageListResponse = serde_json::from_str(&response_text)
            .map_err(|e| Error::Serialization(format!("解析消息列表失败: {}", e)))?;

        if response.code != 0 {
            return Err(Error::Channel(format!("飞书 API 错误: {}", response.msg)));
        }

        Ok(response)
    }

    /// 获取单条消息详情
    ///
    /// 根据消息 ID 获取消息的完整详情
    ///
    /// # 参数说明
    /// * `message_id` - 消息 ID
    ///
    /// # 返回值
    /// 消息详情
    pub async fn get_message(&self, message_id: &str) -> Result<super::handlers::FeishuMessageItem> {
        let token = self.get_access_token().await?;
        let url = format!("{}/im/v1/messages/{}", self.base_url, message_id);

        let response = self.http_client
            .get(&url)
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await
            .map_err(|e| Error::Network(format!("获取消息详情失败: {}", e)))?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(Error::Channel(format!("获取消息详情失败: {}", error_text)));
        }

        let response_text = response.text().await
            .map_err(|e| Error::Network(format!("解析消息详情响应失败: {}", e)))?;

        #[derive(Deserialize)]
        struct MessageResponse {
            code: i64,
            msg: String,
            data: Option<MessageData>,
        }

        #[derive(Deserialize)]
        struct MessageData {
            #[serde(rename = "items")]
            items: Option<Vec<super::handlers::FeishuMessageItem>>,
        }

        let response: MessageResponse = serde_json::from_str(&response_text)
            .map_err(|e| Error::Serialization(format!("解析消息详情失败: {}", e)))?;

        if response.code != 0 {
            return Err(Error::Channel(format!("飞书 API 错误: {}", response.msg)));
        }

        response.data
            .and_then(|d| d.items.and_then(|mut items| items.pop()))
            .ok_or_else(|| Error::Channel("响应中未包含消息详情".to_string()))
    }

    /// 获取消息内容
    ///
    /// 获取指定消息的完整内容（包括 text, post 等）
    ///
    /// # 参数说明
    /// * `message_id` - 消息 ID
    /// * `message_type` - 消息类型（text, post, image 等）
    ///
    /// # 返回值
    /// 消息内容字符串
    pub async fn get_message_content(&self, message_id: &str, message_type: &str) -> Result<String> {
        let token = self.get_access_token().await?;
        let url = format!("{}/im/v1/messages/{}/content", self.base_url, message_id);

        let response = self.http_client
            .get(&url)
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await
            .map_err(|e| Error::Network(format!("获取消息内容失败: {}", e)))?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(Error::Channel(format!("获取消息内容失败: {}", error_text)));
        }

        let response_text = response.text().await
            .map_err(|e| Error::Network(format!("解析消息内容响应失败: {}", e)))?;

        #[derive(Deserialize)]
        struct ContentResponse {
            code: i64,
            msg: String,
            data: Option<ContentData>,
        }

        #[derive(Deserialize)]
        struct ContentData {
            #[serde(rename = "content")]
            content: String,
        }

        let response: ContentResponse = serde_json::from_str(&response_text)
            .map_err(|e| Error::Serialization(format!("解析消息内容失败: {}", e)))?;

        if response.code != 0 {
            return Err(Error::Channel(format!("飞书 API 错误: {}", response.msg)));
        }

        response.data
            .map(|d| d.content)
            .ok_or_else(|| Error::Channel("响应中未包含消息内容".to_string()))
    }

    /// 发送已读回执
    pub async fn mark_message_read(&self, message_id: &str) -> Result<()> {
        let token = self.get_access_token().await?;
        let url = format!("{}/im/v1/messages/{}/read", self.base_url, message_id);

        #[derive(Serialize)]
        struct MarkReadRequest {
            #[serde(rename = "read_time")]
            read_time: String,
        }

        let response = self.http_client
            .post(&url)
            .header("Authorization", format!("Bearer {}", token))
            .json(&MarkReadRequest {
                read_time: chrono::Utc::now().to_rfc3339(),
            })
            .send()
            .await
            .map_err(|e| Error::Network(format!("发送已读回执失败: {}", e)))?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            warn!(message_id = %message_id, error = %error_text, "发送已读回执失败");
        }

        Ok(())
    }

    /// 获取 WebSocket 长链接 URL
    ///
    /// 飞书使用 WebSocket 长连接接收消息事件
    ///
    /// # 返回值
    /// WebSocket URL
    pub async fn get_websocket_url(&self) -> Result<String> {
        // 使用特殊的端点获取 WebSocket URL，不依赖 access_token，而是使用 AppID 和 AppSecret
        // SDK 参考: https://github.com/larksuite/oapi-sdk-go/blob/v3_main/ws/client.go#L236
        // Endpoint: /callback/ws/endpoint
        let url = "https://open.feishu.cn/callback/ws/endpoint";

        #[derive(Serialize)]
        struct WsAuthRequest {
            #[serde(rename = "AppID")]
            app_id: String,
            #[serde(rename = "AppSecret")]
            app_secret: String,
        }

        let request_body = WsAuthRequest {
            app_id: self.credentials.app_id.clone(),
            app_secret: self.credentials.app_secret.clone(),
        };

        let response = self.http_client
            .post(url)
            .header("Content-Type", "application/json; charset=utf-8")
            .json(&request_body)
            .send()
            .await
            .map_err(|e| Error::Network(format!("获取 WebSocket URL 失败: {}", e)))?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(Error::Network(format!("获取 WebSocket URL 失败: {}, url: {}", error_text, url)));
        }

        let response_text = response.text().await
            .map_err(|e| Error::Network(format!("解析 WebSocket URL 响应失败: {}", e)))?;

        #[derive(Deserialize)]
        struct WsUrlResponse {
            code: i64,
            msg: String,
            data: Option<WsUrlData>,
        }

        #[derive(Deserialize)]
        struct WsUrlData {
            #[serde(rename = "URL")]
            url: String,
        }

        let ws_response: WsUrlResponse = serde_json::from_str(&response_text)
            .map_err(|e| Error::Serialization(format!("解析 WebSocket URL 失败: {}", e)))?;

        if ws_response.code != 0 {
            return Err(Error::Channel(format!("获取 WebSocket URL 失败: {}", ws_response.msg)));
        }

        ws_response.data
            .map(|d| d.url)
            .ok_or_else(|| Error::Channel("响应中未包含 WebSocket URL".to_string()))
    }

    /// 连接 WebSocket
    pub async fn connect_websocket(
        &self,
        url: &str,
    ) -> Result<(
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>
        >,
        tokio_tungstenite::tungstenite::handshake::client::Response,
    )> {
        use tokio_tungstenite::connect_async;

        let (ws, response) = connect_async(url).await
            .map_err(|e| Error::Network(format!("连接 WebSocket 失败: {}", e)))?;

        Ok((ws, response))
    }
}

/// 飞书 API 响应基础结构
#[derive(Debug, Deserialize)]
struct ApiResponse<T> {
    code: i64,
    msg: String,
    data: Option<T>,
}
