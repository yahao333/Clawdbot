//! 命令行参数解析模块
//!
//! 本模块使用标准库解析命令行参数，支持以下参数：
/// - `--config` / `-c`: 配置文件路径
/// - `--verbose` / `-v`: 开启详细日志
/// - `--version` / `-V`: 显示版本号
/// - `--help` / `-h`: 显示帮助信息
///
/// # 使用示例
/// ```bash
/// clawdbot --config config.toml --verbose
/// clawdbot -c config.json5 -v
/// ```

use std::path::PathBuf;

/// CLI 参数结构体
///
/// 包含所有支持的命令行参数
#[derive(Debug, Clone)]
pub struct CliArgs {
    /// 配置文件路径
    pub config: PathBuf,
    /// 是否开启详细日志
    pub verbose: bool,
    /// 是否显示版本号
    pub version: bool,
    /// 是否显示帮助信息
    pub help: bool,
}

impl Default for CliArgs {
    /// 默认参数值
    ///
    /// 默认配置文件：`clawdbot.toml`
    /// 默认不开启详细日志
    fn default() -> Self {
        Self {
            config: PathBuf::from("clawdbot.toml"),
            verbose: false,
            version: false,
            help: false,
        }
    }
}

/// 解析命令行参数
///
/// # 参数说明
/// * `args` - 命令行参数列表（通常是 std::env::args()）
///
/// # 返回值
/// 解析成功返回 `CliArgs`，解析失败返回错误信息
///
/// # 示例
/// ```rust
/// let args = CliArgs::parse();
/// ```
pub fn parse_args(args: &[String]) -> Result<CliArgs, String> {
    // 如果没有参数，返回默认配置
    if args.is_empty() {
        return Ok(CliArgs::default());
    }

    let mut result = CliArgs::default();
    let mut iter = args.iter();

    // 跳过程序名
    let _program = iter.next();

    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--config" | "-c" => {
                if let Some(path) = iter.next() {
                    result.config = PathBuf::from(path);
                } else {
                    return Err("'--config' 需要指定配置文件路径".to_string());
                }
            }
            "--verbose" | "-v" => {
                result.verbose = true;
            }
            "--version" | "-V" => {
                result.version = true;
            }
            "--help" | "-h" => {
                result.help = true;
            }
            _ => {
                // 未知参数
                return Err(format!("未知参数: {}", arg));
            }
        }
    }

    Ok(result)
}
