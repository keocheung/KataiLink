mod config;
mod model;
mod provider;

use std::{
    collections::HashMap,
    env,
    ffi::OsStr,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow, bail};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use teloxide::{
    dispatching::Dispatcher,
    dptree,
    payloads::SendMessageSetters,
    prelude::*,
    requests::Requester,
    types::{InlineKeyboardButton, InlineKeyboardMarkup, InputFile, MessageId},
};
use tokio::{
    process::Command,
    sync::{Mutex, mpsc},
    time::sleep,
};
use url::Url;

use crate::{
    config::{Config, TaskConfig, load_config},
    model::MediaInfo,
    provider::{AiProvider, build_ai_provider},
};

#[derive(Debug, Clone)]
struct LinkOperation {
    source: PathBuf,
    target: PathBuf,
}

#[derive(Debug, Clone)]
struct PendingJob {
    source_video: PathBuf,
    task_index: usize,
    media: MediaInfo,
    operations: Vec<LinkOperation>,
}

#[derive(Debug, Clone)]
struct FileEvent {
    task_index: usize,
    path: PathBuf,
}

struct AppState {
    config: Config,
    ai_provider: Arc<dyn AiProvider>,
    allowed_chat_id: ChatId,
    pending_jobs: Mutex<HashMap<u64, PendingJob>>,
    id_seq: AtomicU64,
    tmdb_api_key: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    init_logger();

    let config = load_config()?;

    let ai_provider = build_ai_provider(&config.ai_provider)?;

    let chat_id_raw = env::var("KATAILINK_CHAT_ID")
        .context("缺少环境变量 KATAILINK_CHAT_ID（仅允许这个 chat id 与机器人交互）")?;
    let allowed_chat_id = ChatId(
        chat_id_raw
            .parse::<i64>()
            .with_context(|| format!("KATAILINK_CHAT_ID 不是合法数字: {chat_id_raw}"))?,
    );

    for (idx, task) in config.tasks.iter().enumerate() {
        if !task.watch_path.is_dir() {
            bail!(
                "tasks[{idx}] watch_path 不存在或不是目录: {}",
                task.watch_path.display()
            );
        }
        if !task.dest_path.exists() {
            std::fs::create_dir_all(&task.dest_path).with_context(|| {
                format!(
                    "创建 tasks[{idx}] dest_path 失败: {}",
                    task.dest_path.display()
                )
            })?;
        }
    }

    let bot = Bot::from_env();
    let tmdb_api_key = env::var("TMDB_API_KEY")
        .ok()
        .filter(|v| !v.trim().is_empty());

    let state = Arc::new(AppState {
        config,
        ai_provider,
        allowed_chat_id,
        pending_jobs: Mutex::new(HashMap::new()),
        id_seq: AtomicU64::new(1),
        tmdb_api_key,
    });

    let (tx, rx) = mpsc::unbounded_channel::<FileEvent>();
    let _watchers = init_watchers(&state.config.tasks, tx)?;
    log::info!("目录监听已启动，共 {} 个任务", state.config.tasks.len());

    let event_bot = bot.clone();
    let event_state = state.clone();
    tokio::spawn(async move {
        if let Err(err) = file_event_loop(event_bot, event_state, rx).await {
            log::error!("文件事件处理循环异常退出: {err:#}");
        }
    });

    let handler = dptree::entry()
        .branch(Update::filter_message().endpoint(handle_message))
        .branch(Update::filter_callback_query().endpoint(handle_callback_query));

    Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![state])
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;

    Ok(())
}

fn init_logger() {
    let mut builder =
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"));
    builder.target(env_logger::Target::Stdout).init();
}

fn init_watchers(
    tasks: &[TaskConfig],
    tx: mpsc::UnboundedSender<FileEvent>,
) -> Result<Vec<RecommendedWatcher>> {
    let mut watchers = Vec::with_capacity(tasks.len());

    for (task_index, task) in tasks.iter().enumerate() {
        let tx_for_task = tx.clone();
        let mut watcher =
            notify::recommended_watcher(move |event: notify::Result<Event>| match event {
                Ok(ev) => {
                    if !matches!(
                        ev.kind,
                        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Any
                    ) {
                        return;
                    }

                    for path in ev.paths {
                        let _ = tx_for_task.send(FileEvent { task_index, path });
                    }
                }
                Err(err) => {
                    log::error!("notify watcher error: {err}");
                }
            })?;

        watcher.watch(&task.watch_path, RecursiveMode::Recursive)?;
        watchers.push(watcher);
        log::info!(
            "任务 {} 开始监听: {} -> {} (confirm={})",
            task_index,
            task.watch_path.display(),
            task.dest_path.display(),
            task.confirm
        );
    }

    Ok(watchers)
}

async fn file_event_loop(
    bot: Bot,
    state: Arc<AppState>,
    mut rx: mpsc::UnboundedReceiver<FileEvent>,
) -> Result<()> {
    let mut dedup: HashMap<PathBuf, Instant> = HashMap::new();

    while let Some(file_event) = rx.recv().await {
        let now = Instant::now();
        if let Some(last_seen) = dedup.get(&file_event.path) {
            if now.duration_since(*last_seen) < Duration::from_secs(3) {
                continue;
            }
        }
        dedup.insert(file_event.path.clone(), now);

        // 等待短暂写入完成，避免处理到尚未落盘完整的文件。
        sleep(Duration::from_millis(800)).await;

        if let Err(err) = process_file_event(&bot, &state, file_event).await {
            log::warn!("处理文件事件失败: {err:#}");
        }

        dedup.retain(|_, ts| now.duration_since(*ts) < Duration::from_secs(60));
    }

    Ok(())
}

async fn process_file_event(bot: &Bot, state: &Arc<AppState>, event: FileEvent) -> Result<()> {
    let task = state
        .config
        .tasks
        .get(event.task_index)
        .ok_or_else(|| anyhow!("无效 task_index: {}", event.task_index))?;

    if !event.path.is_file() {
        return Ok(());
    }

    if !is_video_file(&event.path) {
        return Ok(());
    }

    let file_name = event
        .path
        .file_name()
        .and_then(OsStr::to_str)
        .ok_or_else(|| anyhow!("文件名含非 UTF-8，暂不支持: {}", event.path.display()))?;

    log::info!(
        "检测到新视频文件: task={} path={}",
        event.task_index,
        event.path.display()
    );

    let media = state
        .ai_provider
        .identify(&state.config.prompt, file_name)
        .await
        .with_context(|| format!("AI 识别失败: {file_name}"))?;

    log::info!(
        "AI 识别结果: name={} year={} tmdb_id={} season={:?} episode={:?}",
        media.original_name,
        media.year,
        media.tmdb_id,
        media.season,
        media.episode
    );

    let operations = build_link_operations(task, &event.path, &media)?;
    if operations.is_empty() {
        log::warn!("没有可执行的硬链接操作: {}", event.path.display());
        return Ok(());
    }

    if task.confirm {
        let request_id = state.id_seq.fetch_add(1, Ordering::Relaxed);
        let pending = PendingJob {
            source_video: event.path.clone(),
            task_index: event.task_index,
            media: media.clone(),
            operations: operations.clone(),
        };

        state.pending_jobs.lock().await.insert(request_id, pending);

        let summary = render_pending_summary(request_id, &event.path, task, &media, &operations);
        let keyboard = InlineKeyboardMarkup::new(vec![vec![
            InlineKeyboardButton::callback("确认执行", format!("confirm:{request_id}")),
            InlineKeyboardButton::callback("取消", format!("reject:{request_id}")),
        ]]);

        if let Some(cover_url) = fetch_tmdb_cover_url(&state, &media).await {
            let cover_url = Url::parse(&cover_url).ok();
            if let Some(cover_url) = cover_url {
                bot.send_photo(state.allowed_chat_id, InputFile::url(cover_url))
                    .caption(summary)
                    .reply_markup(keyboard)
                    .await
                    .context("发送带封面确认消息失败")?;
            } else {
                bot.send_message(state.allowed_chat_id, summary.clone())
                    .reply_markup(keyboard.clone())
                    .await
                    .context("封面 URL 无效，回退发送确认消息失败")?;
            }
        } else {
            bot.send_message(state.allowed_chat_id, summary)
                .reply_markup(keyboard)
                .await
                .context("发送确认消息失败")?;
        }

        log::info!("已发送确认请求: id={request_id}");
        return Ok(());
    }

    execute_link_operations(&operations)?;
    let done_text = format!(
        "已执行硬链接: {}\n目标条目数: {}\nTMDB ID: {}",
        event.path.display(),
        operations.len(),
        media.tmdb_id
    );
    let _ = bot.send_message(state.allowed_chat_id, done_text).await;
    Ok(())
}

async fn handle_message(bot: Bot, msg: Message, state: Arc<AppState>) -> Result<()> {
    if msg.chat.id != state.allowed_chat_id {
        log::warn!(
            "拒绝未授权聊天: got={}, expected={}",
            msg.chat.id,
            state.allowed_chat_id
        );
        return Ok(());
    }

    let text = msg.text().unwrap_or_default().trim();
    if text == "/start" || text == "/help" {
        let help = "KataiLink 已运行。\n- 自动监听目录\n- confirm=true 时会推送确认按钮\n- /pending 查看待确认任务数";
        bot.send_message(msg.chat.id, help).await?;
        return Ok(());
    }

    if text == "/pending" {
        let count = state.pending_jobs.lock().await.len();
        bot.send_message(msg.chat.id, format!("待确认任务数: {count}"))
            .await?;
        return Ok(());
    }

    Ok(())
}

async fn handle_callback_query(bot: Bot, q: CallbackQuery, state: Arc<AppState>) -> Result<()> {
    let Some(message) = &q.message else {
        return Ok(());
    };

    if message.chat().id != state.allowed_chat_id {
        log::warn!(
            "拒绝未授权 callback: got={}, expected={}",
            message.chat().id,
            state.allowed_chat_id
        );
        return Ok(());
    }

    let Some(data) = q.data.as_deref() else {
        return Ok(());
    };

    let (action, id_part) = data
        .split_once(':')
        .ok_or_else(|| anyhow!("非法 callback data: {data}"))?;
    let request_id: u64 = id_part
        .parse()
        .with_context(|| format!("非法 request id: {id_part}"))?;

    let maybe_job = state.pending_jobs.lock().await.remove(&request_id);

    match (action, maybe_job) {
        ("confirm", Some(job)) => {
            let result = execute_link_operations(&job.operations);
            match result {
                Ok(()) => {
                    if let Err(err) = update_callback_buttons(
                        &bot,
                        message.chat().id,
                        message.id(),
                        request_id,
                        "✅ 已确认执行",
                    )
                    .await
                    {
                        log::warn!("更新确认按钮状态失败: {err:#}");
                    }
                    bot.answer_callback_query(q.id).text("已执行").await?;
                    bot.send_message(
                        state.allowed_chat_id,
                        format!(
                            "执行完成: {}\n目标条目数: {}\nTMDB ID: {}",
                            job.source_video.display(),
                            job.operations.len(),
                            job.media.tmdb_id
                        ),
                    )
                    .await?;
                }
                Err(err) => {
                    bot.answer_callback_query(q.id).text("执行失败").await?;
                    bot.send_message(state.allowed_chat_id, format!("执行失败: {err:#}"))
                        .await?;
                }
            }
        }
        ("reject", Some(job)) => {
            if let Err(err) = update_callback_buttons(
                &bot,
                message.chat().id,
                message.id(),
                request_id,
                "❌ 已取消",
            )
            .await
            {
                log::warn!("更新取消按钮状态失败: {err:#}");
            }
            bot.answer_callback_query(q.id).text("已取消").await?;
            bot.send_message(
                state.allowed_chat_id,
                format!(
                    "已取消任务: {} (task={})",
                    job.source_video.display(),
                    job.task_index
                ),
            )
            .await?;
        }
        ("done", _) => {
            bot.answer_callback_query(q.id).text("该任务已处理").await?;
        }
        (_, None) => {
            bot.answer_callback_query(q.id)
                .text("任务不存在或已处理")
                .await?;
        }
        _ => {
            bot.answer_callback_query(q.id).text("未知操作").await?;
        }
    }

    Ok(())
}

async fn fetch_tmdb_cover_url(state: &AppState, media: &MediaInfo) -> Option<String> {
    let api_key = state.tmdb_api_key.as_ref()?;
    let media_type = if media.is_tv() { "tv" } else { "movie" };
    let endpoint = format!(
        "https://api.themoviedb.org/3/{media_type}/{}?api_key={api_key}&language=zh-CN",
        media.tmdb_id
    );

    let output = match Command::new("curl")
        .arg("-fsSL")
        .arg(endpoint)
        .output()
        .await
    {
        Ok(result) => result,
        Err(err) => {
            log::warn!("调用 curl 请求 TMDB 失败: {err}");
            return None;
        }
    };

    if !output.status.success() {
        log::warn!("TMDB 请求失败，curl exit code: {:?}", output.status.code());
        return None;
    }

    let json: serde_json::Value = match serde_json::from_slice(&output.stdout) {
        Ok(value) => value,
        Err(err) => {
            log::warn!("解析 TMDB 返回失败: {err}");
            return None;
        }
    };

    let Some(poster_path) = json.get("poster_path").and_then(|v| v.as_str()) else {
        return None;
    };

    Some(format!("https://image.tmdb.org/t/p/w500{poster_path}"))
}

async fn update_callback_buttons(
    bot: &Bot,
    chat_id: ChatId,
    message_id: MessageId,
    request_id: u64,
    label: &str,
) -> Result<()> {
    let keyboard = InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::callback(
        label.to_string(),
        format!("done:{request_id}"),
    )]]);

    bot.edit_message_reply_markup(chat_id, message_id)
        .reply_markup(keyboard)
        .await?;

    Ok(())
}

fn build_link_operations(
    task: &TaskConfig,
    source_video: &Path,
    media: &MediaInfo,
) -> Result<Vec<LinkOperation>> {
    let ext = source_video
        .extension()
        .and_then(OsStr::to_str)
        .ok_or_else(|| anyhow!("视频文件扩展名为空: {}", source_video.display()))?;

    let safe_title = sanitize_name(&media.original_name);
    let (container_dir, season_dir, file_stem) = if media.is_tv() {
        let season = media.season.ok_or_else(|| anyhow!("season 缺失"))?;
        let episode = media.episode.ok_or_else(|| anyhow!("episode 缺失"))?;

        (
            format!("{} ({})", safe_title, media.year),
            Some(format!("Season {}", season)),
            format!("{} - S{:02}E{:02}", safe_title, season, episode),
        )
    } else {
        (
            format!("{} ({})", safe_title, media.year),
            None,
            safe_title.to_string(),
        )
    };

    let mut target_dir = task.dest_path.join(container_dir);
    if let Some(season_dir) = season_dir {
        target_dir = target_dir.join(season_dir);
    }
    let video_target = target_dir.join(format!("{file_stem}.{ext}"));

    let mut operations = vec![LinkOperation {
        source: source_video.to_path_buf(),
        target: video_target,
    }];

    for (subtitle_path, language_tag) in find_matching_subtitles(source_video)? {
        let sub_ext = subtitle_path
            .extension()
            .and_then(OsStr::to_str)
            .ok_or_else(|| anyhow!("字幕扩展名缺失: {}", subtitle_path.display()))?;

        let subtitle_name = match language_tag {
            Some(lang) => format!("{file_stem}.{lang}.{sub_ext}"),
            None => format!("{file_stem}.{sub_ext}"),
        };

        operations.push(LinkOperation {
            source: subtitle_path,
            target: target_dir.join(subtitle_name),
        });
    }

    Ok(operations)
}

fn find_matching_subtitles(source_video: &Path) -> Result<Vec<(PathBuf, Option<String>)>> {
    let parent = source_video
        .parent()
        .ok_or_else(|| anyhow!("source_video 没有父目录: {}", source_video.display()))?;
    let base_stem = source_video
        .file_stem()
        .and_then(OsStr::to_str)
        .ok_or_else(|| anyhow!("source_video 文件名不可解析: {}", source_video.display()))?;

    let mut result = Vec::new();

    for entry in std::fs::read_dir(parent)? {
        let entry = entry?;
        let path = entry.path();

        if path == source_video || !path.is_file() {
            continue;
        }

        if !is_subtitle_file(&path) {
            continue;
        }

        let Some(stem) = path.file_stem().and_then(OsStr::to_str) else {
            continue;
        };

        if stem == base_stem {
            result.push((path, None));
            continue;
        }

        let Some(suffix) = stem.strip_prefix(base_stem) else {
            continue;
        };

        let suffix = suffix.trim_start_matches('.');
        if suffix.is_empty() {
            result.push((path, None));
            continue;
        }

        let language_tag = map_language_tag(suffix);
        result.push((path, language_tag));
    }

    Ok(result)
}

fn map_language_tag(raw: &str) -> Option<String> {
    let normalized = raw.trim().to_ascii_lowercase();
    let mapped = match normalized.as_str() {
        "chs" | "sc" | "zh-cn" | "zh_hans" | "zh-hans" => "zh-Hans",
        "cht" | "tc" | "zh-tw" | "zh_hant" | "zh-hant" => "zh-Hant",
        "jp" | "jpn" | "ja" => "ja",
        "eng" | "en" => "en",
        _ => normalized.as_str(),
    };

    if mapped.is_empty() {
        None
    } else {
        Some(mapped.to_string())
    }
}

fn execute_link_operations(operations: &[LinkOperation]) -> Result<()> {
    for op in operations {
        let parent = op
            .target
            .parent()
            .ok_or_else(|| anyhow!("目标文件无父目录: {}", op.target.display()))?;
        std::fs::create_dir_all(parent)
            .with_context(|| format!("创建目录失败: {}", parent.display()))?;

        if op.target.exists() {
            if is_same_file(&op.source, &op.target).unwrap_or(false) {
                log::info!("目标已存在且是同一文件，跳过: {}", op.target.display());
                continue;
            }
            bail!("目标已存在，拒绝覆盖: {}", op.target.display());
        }

        std::fs::hard_link(&op.source, &op.target).with_context(|| {
            format!(
                "硬链接失败: {} -> {}",
                op.source.display(),
                op.target.display()
            )
        })?;

        log::info!(
            "硬链接完成: {} -> {}",
            op.source.display(),
            op.target.display()
        );
    }

    Ok(())
}

fn is_same_file(a: &Path, b: &Path) -> std::io::Result<bool> {
    let ma = std::fs::metadata(a)?;
    let mb = std::fs::metadata(b)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(ma.ino() == mb.ino() && ma.dev() == mb.dev())
    }

    #[cfg(not(unix))]
    {
        Ok(false)
    }
}

fn render_pending_summary(
    request_id: u64,
    source_video: &Path,
    task: &TaskConfig,
    media: &MediaInfo,
    operations: &[LinkOperation],
) -> String {
    let mut lines = vec![
        "KataiLink 待确认任务".to_string(),
        format!("ID: {}", request_id),
        format!("来源: {}", source_video.display()),
        format!("目标根目录: {}", task.dest_path.display()),
        format!("标题: {}", media.original_name),
        format!("年份: {}", media.year),
        format!("TMDB ID: {}", media.tmdb_id),
    ];

    if let (Some(season), Some(episode)) = (media.season, media.episode) {
        lines.push(format!("集数: S{:02}E{:02}", season, episode));
    }

    lines.push(format!("操作数: {}", operations.len()));

    for op in operations {
        lines.push(format!(
            "{} -> {}",
            op.source.display(),
            op.target.display()
        ));
    }

    lines.join("\n")
}

fn sanitize_name(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for c in raw.chars() {
        let mapped = match c {
            '/' => '／',
            '\\' => '＼',
            ':' => '：',
            '*' => '＊',
            '?' => '？',
            '"' => '＂',
            '<' => '＜',
            '>' => '＞',
            '|' => '｜',
            _ => c,
        };
        out.push(mapped);
    }

    out = out.trim().to_string();
    if out.is_empty() {
        "Unknown".to_string()
    } else {
        out
    }
}

fn is_video_file(path: &Path) -> bool {
    let Some(ext) = path.extension().and_then(OsStr::to_str) else {
        return false;
    };

    matches!(
        ext.to_ascii_lowercase().as_str(),
        "mkv" | "mp4" | "avi" | "mov" | "wmv" | "flv" | "m4v" | "ts"
    )
}

fn is_subtitle_file(path: &Path) -> bool {
    let Some(ext) = path.extension().and_then(OsStr::to_str) else {
        return false;
    };

    matches!(
        ext.to_ascii_lowercase().as_str(),
        "srt" | "ass" | "ssa" | "sub" | "vtt"
    )
}
