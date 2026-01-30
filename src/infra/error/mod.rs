//! 错误处理模块

/// 错误类型
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("配置错误: {0}")]
    Config(String),

    #[error("网络错误: {0}")]
    Network(String),

    #[error("数据库错误: {0}")]
    Database(String),

    #[error("认证错误: {0}")]
    Auth(String),

    #[error("渠道错误: {0}")]
    Channel(String),

    #[error("AI 错误: {0}")]
    Ai(String),

    #[error("序列化错误: {0}")]
    Serialization(String),

    #[error("IO 错误: {0}")]
    Io(String),

    #[error("未知错误: {0}")]
    Unknown(String),
}

/// 结果类型
pub type Result<T> = std::result::Result<T, Error>;

impl From<&str> for Error {
    fn from(s: &str) -> Self {
        Self::Unknown(s.to_string())
    }
}

impl From<String> for Error {
    fn from(s: String) -> Self {
        Self::Unknown(s)
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e.to_string())
    }
}
