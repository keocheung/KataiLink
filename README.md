# KataiLink

KataiLink 是一个影视文件整理机器人：监听下载目录，识别视频对应的剧集/电影信息，并将文件硬链接到媒体库目录结构，支持 Telegram 二次确认。

## 功能
- 监听多个目录的新视频文件（`notify`）
- 调用 `codex` CLI 识别媒体信息（原名、年份、TMDB ID、季集）
- 自动生成目标路径并执行硬链接
- 自动发现同名字幕并一并硬链接（含语言标签映射）
- Telegram 交互确认（可关闭确认，直接执行）
- 输出运行日志到 stdout

## 目录规则
- 电影：
  - `{dest_path}/{original_name} ({year})/{original_name}.{ext}`
- 剧集：
  - `{dest_path}/{original_name} ({year})/Season {season}/{original_name} - S{season:02}E{episode:02}.{ext}`

说明：非法路径字符会替换为全角字符（如 `/ -> ／`，`: -> ：`）。

## 环境要求
- Rust（建议 stable）
- 已安装并可执行的 `codex` CLI
- Telegram Bot Token

## 配置
默认读取 `./config.yaml`，也可用 `KATAILINK_CONFIG` 指定。

示例：
```yaml
ai_provider: codex-cli
prompt: 识别下列文件名对应的电视节目或电影，结合网络搜索，给出它的原名、年份、TMDB ID，以及在TMDB上对应的季数和集数
tasks:
  - watch_path: /Users/keo/Movies/test
    dest_path: /Users/keo/Movies/links
    confirm: true
```

## 运行
```bash
export TELOXIDE_TOKEN=<your_bot_token>
export KATAILINK_CHAT_ID=<allowed_chat_id>
# 可选：export KATAILINK_CONFIG=/path/to/config.yaml

cargo run
```

## 开发命令
- `cargo check`：快速检查编译
- `cargo fmt`：格式化代码
- `cargo test`：运行测试（如有）

## Telegram 交互
- `/start` 或 `/help`：查看说明
- `/pending`：查看待确认任务数量

仅 `KATAILINK_CHAT_ID` 对应的 chat 可操作机器人。

## 当前限制
- AI Provider 目前仅实现 `codex-cli`
- AI 输出格式错误时会最多重试 3 次
- `codex` 命令执行失败（非 0）会直接报错，不重试
