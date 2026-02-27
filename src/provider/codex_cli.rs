use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use tokio::{
    process::Command,
    time::{Duration, sleep},
};

use crate::{model::MediaInfo, provider::AiProvider};

pub struct CodexCliProvider;

#[async_trait]
impl AiProvider for CodexCliProvider {
    async fn identify(&self, prompt: &str, file_name: &str) -> Result<MediaInfo> {
        identify_with_codex_cli(prompt, file_name).await
    }
}

async fn identify_with_codex_cli(prompt: &str, file_name: &str) -> Result<MediaInfo> {
    const MAX_ATTEMPTS: usize = 3;
    let final_prompt = format!(
        "{prompt}\n\n文件名: {file_name}\n\n严格只输出 JSON，对象字段为 original_name, year, tmdb_id, season, episode。"
    );

    let mut last_err: Option<anyhow::Error> = None;

    for attempt in 1..=MAX_ATTEMPTS {
        let output = Command::new("codex")
            .arg("exec")
            .arg(&final_prompt)
            .output()
            .await
            .context("调用 codex CLI 失败")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("codex CLI 返回非 0 状态: {}", stderr.trim());
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        match parse_media_info_from_output(&stdout) {
            Ok(parsed) => return Ok(parsed),
            Err(err) => {
                log::warn!(
                    "AI 输出格式错误，准备重试: attempt={}/{} file={} err={:#}",
                    attempt,
                    MAX_ATTEMPTS,
                    file_name,
                    err
                );
                last_err = Some(err);
                if attempt < MAX_ATTEMPTS {
                    sleep(Duration::from_millis(300)).await;
                }
            }
        }
    }

    Err(last_err.unwrap_or_else(|| anyhow!("AI 识别失败且未返回可解析结果")))
}

fn parse_media_info_from_output(stdout: &str) -> Result<MediaInfo> {
    let json_str = extract_json_object(stdout)
        .ok_or_else(|| anyhow!("codex 输出不含 JSON 对象: {}", stdout.trim()))?;

    let parsed: MediaInfo =
        serde_json::from_str(json_str).with_context(|| format!("JSON 解析失败: {}", json_str))?;

    if parsed.original_name.trim().is_empty() {
        bail!("识别结果 original_name 为空");
    }

    Ok(parsed)
}

fn extract_json_object(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    if end <= start {
        return None;
    }
    Some(&text[start..=end])
}
