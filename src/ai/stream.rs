//! AI 流式响应模块
//!
//! 提供 SSE (Server-Sent Events) 流式解析功能
//!
//! # OpenAI SSE 格式
//! ```
//! data: {"id":"...","object":"chat.completion.chunk","choices":[{"delta":{"content":"Hello"}}]}
//! data: [DONE]
//! ```
//!
//! # 使用示例
//! ```rust
//! use clawdbot::ai::stream::{StreamParser, parse_sse_line};
//!
//! // 解析单行 SSE 数据
//! if let Some(data) = parse_sse_line(line) {
//!     println!("{}", data);
//! }
//! ```

use std::pin::Pin;
use std::task::{Context, Poll};
use std::io::{Error, ErrorKind};
use tokio::io::{AsyncRead, AsyncBufReadExt, BufReader};
use tracing::debug;

/// SSE 事件类型
#[derive(Debug, Clone, PartialEq)]
pub enum SseEvent {
    /// 数据事件
    Data(String),
    /// 完成事件
    Done,
    /// 错误事件
    Error(String),
    /// 空行（忽略）
    Ignore,
}

/// 解析 SSE 行
///
/// # 参数说明
/// * `line` - 原始行数据
///
/// # 返回值
/// 解析后的事件
pub fn parse_sse_line(line: &str) -> SseEvent {
    // 忽略空行
    if line.is_empty() {
        return SseEvent::Ignore;
    }

    // 忽略以冒号开头的注释行
    if line.starts_with(':') {
        return SseEvent::Ignore;
    }

    // 解析事件类型
    if let Some(data) = line.strip_prefix("data:") {
        let data = data.trim_start();
        if data == "[DONE]" {
            return SseEvent::Done;
        }
        // 尝试解析 JSON
        if let Some(text) = extract_content_from_json(data) {
            return SseEvent::Data(text);
        }
        // JSON 解析失败，返回原始数据
        return SseEvent::Data(data.to_string());
    }

    SseEvent::Ignore
}

/// 流式响应块
///
/// 包含流式响应的一个数据块
#[derive(Debug, Clone)]
pub struct StreamChunk {
    /// 块内容（文本增量）
    pub content: String,
    /// 是否完成
    pub done: bool,
    /// 完成原因（可选）
    pub finish_reason: Option<String>,
    /// 角色（首个块可能包含）
    pub role: Option<String>,
}

impl StreamChunk {
    /// 创建新块
    pub fn new(content: String, done: bool) -> Self {
        Self {
            content,
            done,
            finish_reason: None,
            role: None,
        }
    }

    /// 创建完成块
    pub fn done(finish_reason: Option<String>) -> Self {
        Self {
            content: String::new(),
            done: true,
            finish_reason,
            role: None,
        }
    }

    /// 检查是否为空块
    pub fn is_empty(&self) -> bool {
        self.content.is_empty() && !self.done
    }
}

/// SSE 流解析器
///
/// 用于解析 Server-Sent Events 格式的流式响应
#[derive(Debug)]
pub struct SseStreamReader {
    /// 内部缓冲区
    buffer: String,
    /// 是否已结束
    finished: bool,
}

impl SseStreamReader {
    /// 创建新的 SSE 流解析器
    pub fn new() -> Self {
        Self {
            buffer: String::new(),
            finished: false,
        }
    }

    /// 解析 SSE 行
    ///
    /// # 参数说明
    /// * `line` - 原始行数据
    ///
    /// # 返回值
    /// 解析后的事件
    pub fn parse_line(line: &str) -> SseEvent {
        parse_sse_line(line)
    }

    /// 检查是否已结束
    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// 标记为结束
    pub fn set_finished(&mut self) {
        self.finished = true;
    }
}

impl Default for SseStreamReader {
    fn default() -> Self {
        Self::new()
    }
}

/// 从 JSON 中提取 content 字段
///
/// 处理 OpenAI 流式响应的 JSON 格式
fn extract_content_from_json(json: &str) -> Option<String> {
    // 尝试查找 "content": "..." 模式
    if let Some(start) = json.find("\"content\":") {
        let after_content = &json[start + 9..]; // 跳过 "content":
        let after_content = after_content.trim_start();

        // 处理字符串
        if after_content.starts_with('"') {
            let mut result = String::new();
            let mut chars = after_content[1..].chars().peekable();

            while let Some(c) = chars.next() {
                if c == '\\' {
                    // 处理转义字符
                    if let Some(next) = chars.next() {
                        match next {
                            'n' => result.push('\n'),
                            'r' => result.push('\r'),
                            't' => result.push('\t'),
                            '\\' => result.push('\\'),
                            '"' => result.push('"'),
                            _ => result.push(next),
                        }
                    }
                } else if c == '"' {
                    // 字符串结束
                    break;
                } else {
                    result.push(c);
                }
            }

            return Some(result);
        }
    }

    // 备选：尝试解析整个 JSON（使用 serde_json 更快但需要依赖）
    #[cfg(feature = "stream_json")]
    {
        use serde::Deserialize;
        #[derive(Deserialize)]
        struct Chunk {
            choices: Vec<Choice>,
        }
        #[derive(Deserialize)]
        struct Choice {
            delta: Option<Delta>,
        }
        #[derive(Deserialize)]
        struct Delta {
            content: Option<String>,
        }

        if let Ok(chunk) = serde_json::from_str::<Chunk>(json) {
            if let Some(choice) = chunk.choices.first() {
                return choice.delta.as_ref()?.content.clone();
            }
        }
    }

    None
}

/// 流式响应读取器
///
/// 将 HTTP 流式响应转换为 AsyncRead
#[derive(Debug)]
pub struct StreamResponseReader {
    /// 内部缓冲区
    buffer: Vec<u8>,
    /// 当前位置
    position: usize,
    /// SSE 解析器
    parser: SseStreamReader,
}

impl StreamResponseReader {
    /// 创建新的流式响应读取器
    pub fn new() -> Self {
        Self {
            buffer: Vec::new(),
            position: 0,
            parser: SseStreamReader::new(),
        }
    }

    /// 处理接收到的数据
    ///
    /// # 参数说明
    /// * `data` - 原始数据块
    pub fn process_data(&mut self, data: &[u8]) {
        // 将数据添加到缓冲区
        self.buffer.extend_from_slice(data);

        // 查找行尾
        while let Some(newline_pos) = find_line_end(&self.buffer, self.position) {
            // 提取一行
            let line = std::str::from_utf8(&self.buffer[self.position..newline_pos])
                .unwrap_or("");

            // 解析 SSE 行
            match SseStreamReader::parse_line(line) {
                SseEvent::Data(content) => {
                    // 将内容转换为文本块格式并添加到缓冲区
                    // 格式: "data: { ... }\n\n" -> "content"
                    if !content.is_empty() {
                        // 添加内容长度前缀（类似 SSE 格式）
                        let prefixed = format!("{}", content);
                        self.buffer.extend_from_slice(prefixed.as_bytes());
                    }
                }
                SseEvent::Done => {
                    // 添加完成标记
                    self.buffer.extend_from_slice(b"[DONE]");
                    self.parser.set_finished();
                    break;
                }
                SseEvent::Error(msg) => {
                    // 添加错误信息
                    let error_msg = format!("ERROR: {}\n", msg);
                    self.buffer.extend_from_slice(error_msg.as_bytes());
                }
                SseEvent::Ignore => {}
            }

            // 移动到下一行
            self.position = newline_pos + 1;
        }

        // 清理已处理的数据
        if self.position > 0 {
            self.buffer.drain(..self.position);
            self.position = 0;
        }
    }

    /// 检查是否已结束
    pub fn is_finished(&self) -> bool {
        self.parser.is_finished() && self.position >= self.buffer.len()
    }
}

impl Default for StreamResponseReader {
    fn default() -> Self {
        Self::new()
    }
}

/// 在缓冲区中查找行尾
fn find_line_end(buffer: &[u8], start: usize) -> Option<usize> {
    for i in start..buffer.len() {
        if buffer[i] == b'\n' {
            return Some(i);
        }
    }
    None
}

impl AsyncRead for StreamResponseReader {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        if self.position >= self.buffer.len() {
            // 没有更多数据
            if self.parser.is_finished() {
                return Poll::Ready(Ok(()));
            }
            return Poll::Ready(Ok(()));
        }

        // 复制可用数据到输出缓冲区
        let available = &self.buffer[self.position..];
        let to_read = std::cmp::min(available.len(), buf.remaining());

        if to_read > 0 {
            buf.put_slice(&available[..to_read]);
            self.position += to_read;
        }

        Poll::Ready(Ok(()))
    }
}

/// 文本流生成器
///
/// 用于生成增量文本流
#[derive(Debug)]
pub struct TextStreamGenerator {
    /// 内部文本
    text: String,
    /// 当前位置
    position: usize,
}

impl TextStreamGenerator {
    /// 从文本创建流生成器
    pub fn from_text(text: String) -> Self {
        Self {
            text,
            position: 0,
        }
    }

    /// 检查是否还有更多数据
    pub fn has_more(&self) -> bool {
        self.position < self.text.len()
    }
}

impl AsyncRead for TextStreamGenerator {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        if self.position >= self.text.len() {
            return Poll::Ready(Ok(()));
        }

        let remaining = &self.text[self.position..];
        let to_read = std::cmp::min(remaining.len(), buf.remaining());

        if to_read > 0 {
            buf.put_slice(remaining.as_bytes());
            self.position += to_read;
        }

        Poll::Ready(Ok(()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sse_parse_data_line() {
        let line = r#"data: {"content": "Hello"}"#;
        let event = SseStreamReader::parse_line(line);
        match event {
            SseEvent::Data(content) => {
                assert_eq!(content, "Hello");
            }
            _ => panic!("Expected Data event"),
        }
    }

    #[test]
    fn test_sse_parse_done() {
        let line = "data: [DONE]";
        let event = SseStreamReader::parse_line(line);
        assert!(matches!(event, SseEvent::Done));
    }

    #[test]
    fn test_sse_parse_empty_line() {
        let event = SseStreamReader::parse_line("");
        assert!(matches!(event, SseEvent::Ignore));
    }

    #[test]
    fn test_sse_parse_comment_line() {
        let line = ": This is a comment";
        let event = SseStreamReader::parse_line(line);
        assert!(matches!(event, SseEvent::Ignore));
    }

    #[test]
    fn test_extract_content_simple() {
        let json = r#"{"choices":[{"delta":{"content":"Hello world"}}]}"#;
        let content = extract_content_from_json(json);
        assert_eq!(content, Some("Hello world".to_string()));
    }

    #[test]
    fn test_extract_content_with_escapes() {
        let json = r#"{"choices":[{"delta":{"content":"Hello\nWorld"}}]}"#;
        let content = extract_content_from_json(json);
        assert_eq!(content, Some("Hello\nWorld".to_string()));
    }
}
