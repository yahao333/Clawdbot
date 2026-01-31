//! 飞书媒体下载模块
//!
//! 提供消息媒体文件的下载功能。
//!
//! # 功能
//! 1. 下载图片、音频、文件等媒体
//! 2. 支持临时下载和永久下载
//! 3. 文件类型检测
//!
//! # 使用示例
//! ```rust
//! let downloader = MediaDownloader::new(feishu_client);
//! let image_data = downloader.download_image("img_v2_xxx").await?;
//! ```

use crate::channels::feishu::client::FeishuClient;
use crate::infra::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tracing::{debug, info, warn};

/// 媒体类型
#[derive(Debug, Clone, PartialEq)]
pub enum MediaType {
    /// 图片
    Image,
    /// 音频
    Audio,
    /// 视频
    Video,
    /// 文件
    File,
    /// 表情包
    Sticker,
    /// 未知类型
    Unknown(String),
}

/// 媒体文件信息
#[derive(Debug, Clone)]
pub struct MediaInfo {
    /// 媒体密钥
    pub image_key: String,
    /// 媒体类型
    pub media_type: MediaType,
    /// 文件名
    pub filename: Option<String>,
    /// MIME 类型
    pub mime_type: String,
    /// 文件大小（字节）
    pub size: u64,
    /// 下载 URL
    pub download_url: Option<String>,
}

/// 媒体下载结果
#[derive(Debug, Clone)]
pub struct MediaDownloadResult {
    /// 媒体信息
    pub info: MediaInfo,
    /// 文件内容（内存中）
    pub content: Vec<u8>,
    /// 本地文件路径（如果已保存到磁盘）
    pub local_path: Option<PathBuf>,
}

/// 媒体文件下载器
///
/// 用于从飞书下载各种媒体文件
///
/// # 使用注意
/// - 图片和视频有有效期限制，建议及时下载
/// - 音频和文件可以永久访问
#[derive(Clone)]
pub struct MediaDownloader {
    /// 飞书客户端
    feishu_client: FeishuClient,
    /// 临时文件目录
    temp_dir: PathBuf,
}

impl MediaDownloader {
    /// 创建媒体下载器
    ///
    /// # 参数说明
    /// * `feishu_client` - 飞书客户端
    /// * `temp_dir` - 临时文件存储目录
    pub fn new(feishu_client: FeishuClient, temp_dir: PathBuf) -> Self {
        // 确保目录存在
        if !temp_dir.exists() {
            std::fs::create_dir_all(&temp_dir)
                .expect("创建临时目录失败");
        }

        Self {
            feishu_client,
            temp_dir,
        }
    }

    /// 创建使用默认临时目录的下载器
    pub fn with_default_temp_dir(feishu_client: FeishuClient) -> Self {
        let temp_dir = std::env::temp_dir().join("clawdbot-feishu");
        Self::new(feishu_client, temp_dir)
    }

    /// 下载图片
    ///
    /// 根据 image_key 下载图片文件
    ///
    /// # 参数说明
    /// * `image_key` - 图片密钥（从消息内容中获取）
    ///
    /// # 返回值
    /// 图片数据（PNG/JPG 格式）
    pub async fn download_image(&self, image_key: &str) -> Result<Vec<u8>> {
        debug!(image_key = %image_key, "开始下载图片");

        // 方法 1: 使用 messageResources API 获取下载链接
        let download_url = self.get_download_url(image_key, "image").await?;

        // 下载文件内容
        let content = self.download_from_url(&download_url).await?;

        debug!(image_key = %image_key, size = content.len(), "图片下载成功");

        Ok(content)
    }

    /// 下载音频
    ///
    /// 飞书音频格式为 m4a
    pub async fn download_audio(&self, file_key: &str) -> Result<Vec<u8>> {
        debug!(file_key = %file_key, "开始下载音频");

        let download_url = self.get_download_url(file_key, "audio").await?;
        let content = self.download_from_url(&download_url).await?;

        Ok(content)
    }

    /// 下载视频
    pub async fn download_video(&self, file_key: &str) -> Result<Vec<u8>> {
        debug!(file_key = %file_key, "开始下载视频");

        let download_url = self.get_download_url(file_key, "video").await?;
        let content = self.download_from_url(&download_url).await?;

        Ok(content)
    }

    /// 下载文件
    pub async fn download_file(&self, file_key: &str) -> Result<Vec<u8>> {
        debug!(file_key = %file_key, "开始下载文件");

        let download_url = self.get_download_url(file_key, "file").await?;
        let content = self.download_from_url(&download_url).await?;

        Ok(content)
    }

    /// 下载媒体文件（通用方法）
    ///
    /// 根据消息类型自动选择下载方式
    pub async fn download(&self, image_key: &str, msg_type: &str) -> Result<MediaDownloadResult> {
        let media_type = match msg_type {
            "image" => MediaType::Image,
            "audio" => MediaType::Audio,
            "video" => MediaType::Video,
            "file" => MediaType::File,
            "sticker" => MediaType::Sticker,
            _ => MediaType::Unknown(msg_type.to_string()),
        };

        debug!(image_key = %image_key, media_type = ?media_type, "下载媒体文件");

        let download_url = self.get_download_url(image_key, msg_type).await?;
        let content = self.download_from_url(&download_url).await?;

        // 检测文件类型
        let mime_type = self.detect_mime_type(&content);

        let info = MediaInfo {
            image_key: image_key.to_string(),
            media_type,
            filename: None,
            mime_type,
            size: content.len() as u64,
            download_url: Some(download_url),
        };

        Ok(MediaDownloadResult {
            info,
            content,
            local_path: None,
        })
    }

    /// 获取下载链接
    ///
    /// 调用飞书 messageResources API 获取临时下载链接
    ///
    /// # 参数说明
    /// * `image_key` - 资源密钥
    /// * `msg_type` - 消息类型
    async fn get_download_url(&self, image_key: &str, msg_type: &str) -> Result<String> {
        let path = format!("/im/v1/resources/{}", image_key);

        #[derive(Serialize)]
        struct QueryParams {
            #[serde(rename = "type")]
            msg_type: String,
        }

        #[derive(Deserialize)]
        struct ApiResponse {
            code: i64,
            msg: String,
            data: Option<ResourceData>,
        }

        #[derive(Deserialize)]
        struct ResourceData {
            #[serde(rename = "download_url")]
            download_url: String,
        }

        let response: ApiResponse = self.feishu_client
            .request("GET", &path, Some(QueryParams { msg_type: msg_type.to_string() }))
            .await?;

        if response.code != 0 {
            warn!(code = response.code, msg = response.msg, "获取下载链接失败");
            return Err(Error::Channel(format!("获取下载链接失败: {}", response.msg)));
        }

        response.data
            .map(|d| d.download_url)
            .ok_or_else(|| Error::Channel("响应中未包含下载链接".to_string()))
    }

    /// 从 URL 下载文件
    async fn download_from_url(&self, url: &str) -> Result<Vec<u8>> {
        let response = self.feishu_client.http_client()
            .get(url)
            .send()
            .await
            .map_err(|e| Error::Network(format!("下载文件失败: {}", e)))?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(Error::Network(format!("下载文件失败: {}", error_text)));
        }

        let content = response
            .bytes()
            .await
            .map_err(|e| Error::Network(format!("读取下载内容失败: {}", e)))?
            .to_vec();

        Ok(content)
    }

    /// 检测文件的 MIME 类型
    ///
    /// 根据文件头字节判断真实类型
    fn detect_mime_type(&self, content: &[u8]) -> String {
        if content.len() < 4 {
            return "application/octet-stream".to_string();
        }

        match &content[..4] {
            [0x89, 0x50, 0x4E, 0x47] => "image/png",
            [0xFF, 0xD8, 0xFF, ..] => "image/jpeg",
            [0x47, 0x49, 0x46, 0x38] => "image/gif",
            [0x52, 0x49, 0x46, 0x46] => "image/webp",
            [0x49, 0x44, 0x33, ..] => "audio/mpeg", // ID3
            [0x66, 0x74, 0x79, 0x70] => "video/mp4", // ftyp
            [0x1A, 0x45, 0xDF, 0xA3] => "video/webm", // EBML
            [0x25, 0x50, 0x44, 0x46] => "application/pdf", // PDF
            _ => "application/octet-stream",
        }.to_string()
    }

    /// 将内容保存到临时文件
    ///
    /// # 参数说明
    /// * `image_key` - 文件密钥（用作文件名）
    /// * `content` - 文件内容
    /// * `extension` - 文件扩展名
    ///
    /// # 返回值
    /// 保存的文件路径
    pub async fn save_to_temp_file(&self, image_key: &str, content: &[u8], extension: &str) -> Result<PathBuf> {
        let filename = format!("{}.{}", image_key, extension);
        let file_path = self.temp_dir.join(&filename);

        tokio::fs::write(&file_path, content)
            .await
            .map_err(|e| Error::Io(format!("保存文件失败: {}", e)))?;

        debug!(path = %file_path.display(), "文件已保存到临时目录");

        Ok(file_path)
    }

    /// 清理临时文件
    ///
    /// 删除过期的临时文件
    ///
    /// # 参数说明
    /// * `max_age_seconds` - 最大保留时间（秒）
    pub async fn cleanup_temp_files(&self, max_age_seconds: u64) -> Result<u32> {
        let max_age = std::time::Duration::from_secs(max_age_seconds);
        let mut removed_count = 0;
        let now = std::time::SystemTime::now();

        if let Ok(entries) = std::fs::read_dir(&self.temp_dir) {
            for entry in entries.flatten() {
                if let Ok(metadata) = entry.path().metadata() {
                    if let Ok(modified) = metadata.modified() {
                        if let Ok(elapsed) = now.duration_since(modified) {
                            if elapsed > max_age {
                                if let Err(e) = std::fs::remove_file(entry.path()) {
                                    warn!(path = %entry.path().display(), error = %e, "删除临时文件失败");
                                } else {
                                    removed_count += 1;
                                }
                            }
                        }
                    }
                }
            }
        }

        info!(removed = removed_count, "临时文件清理完成");
        Ok(removed_count)
    }
}
