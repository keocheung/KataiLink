use std::sync::Arc;

use anyhow::{Result, bail};
use async_trait::async_trait;

use crate::model::MediaInfo;

mod codex_cli;

#[async_trait]
pub trait AiProvider: Send + Sync {
    async fn identify(&self, prompt: &str, file_name: &str) -> Result<MediaInfo>;
}

pub fn build_ai_provider(provider_name: &str) -> Result<Arc<dyn AiProvider>> {
    match provider_name.trim() {
        "codex-cli" => Ok(Arc::new(codex_cli::CodexCliProvider)),
        other => bail!("暂不支持 ai_provider={other}，当前支持: codex-cli"),
    }
}
