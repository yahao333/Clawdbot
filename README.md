# Clawdbot

一个高性能跨平台 AI 消息机器人，基于 Rust 实现。

Clawdbot 旨在提供一个灵活、可扩展的即时通讯机器人框架，支持多种 AI 模型提供商（如 OpenAI, Anthropic, Minimax）和多种消息渠道（目前主要支持飞书/Lark）。

## ✨ 特性

- **多模型支持**: 内置 OpenAI, Anthropic, Minimax 等主流大模型接口支持。
- **飞书/Lark 集成**: 深度集成飞书开放平台，支持消息接收、发送、群组管理等功能。
- **高性能**: 基于 Rust 和 Tokio 异步运行时，资源占用低，并发处理能力强。
- **会话管理**: 内置基于 SQLite 的会话持久化存储，支持上下文记忆。
- **Web 管理**: 提供 Web 接口用于状态监控和管理。
- **配置灵活**: 支持 TOML 配置文件和环境变量。

## 🛠️ 项目结构

```
src/
├── ai/          # AI 模型提供商实现 (Anthropic, Minimax, OpenAI)
├── channels/    # 消息渠道适配 (目前主要是 Feishu)
├── cli/         # 命令行入口及参数解析
├── core/        # 核心逻辑 (Agent, 消息队列, 路由引擎, 会话管理)
├── infra/       # 基础设施 (配置, 数据库, 错误处理, 日志)
├── web/         # Web 服务接口
└── main.rs      # 程序入口
```

## 🚀 快速开始

### 前置要求

- Rust 1.75+
- Cargo

### 安装与运行

1. **克隆项目**

```bash
git clone https://gitee.com/your-username/clawdbot.git
cd clawdbot
```

2. **配置环境**

复制示例配置文件（如果有）或创建一个新的 `clawdbot.toml`。同时支持 `.env` 文件加载环境变量。

3. **运行服务**

```bash
# 启动服务
cargo run -- start

# 指定配置文件启动
cargo run -- start --config my_config.toml

# 开启详细日志
cargo run -- start --verbose
```

### 命令行参数

```bash
clawdbot [OPTIONS] [COMMAND]

Commands:
  start    启动 Clawdbot 服务
  check    检查配置文件是否有效
  version  显示版本信息

Options:
  -c, --config <CONFIG>    配置文件路径 [default: clawdbot.toml]
  -v, --verbose            是否启用 verbose 模式（显示 DEBUG 日志）
  -p, --port <PORT>        监听端口 [default: 8080]
      --web-port <PORT>    Web 管理界面端口（0 表示不启动） [default: 3000]
  -h, --help               Print help
```

## 🧪 开发与测试

### 运行测试

```bash
cargo test
```

### 运行示例

项目包含飞书集成的示例代码，位于 `examples/` 目录下。

```bash
# 运行飞书接收消息示例
cargo run --example feishu_receive

# 运行飞书发送消息示例
cargo run --example feishu_send
```

## 📝 依赖概览

主要依赖库包括：
- **Tokio**: 异步运行时
- **Axum**: Web 框架
- **Reqwest**: HTTP 客户端
- **SQLx/Rusqlite**: 数据库交互
- **Tracing**: 日志与追踪
- **Serde**: 序列化与反序列化

## 📄 许可证

[MIT License](LICENSE)
