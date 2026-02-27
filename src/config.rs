use std::{env, path::PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub ai_provider: String,
    pub prompt: String,
    pub tasks: Vec<TaskConfig>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct TaskConfig {
    pub watch_path: PathBuf,
    pub dest_path: PathBuf,
    #[serde(default = "default_confirm")]
    pub confirm: bool,
}

fn default_confirm() -> bool {
    true
}

pub fn load_config() -> Result<Config> {
    let config_path = env::var("KATAILINK_CONFIG").unwrap_or_else(|_| "config.yaml".to_string());
    let config_text = std::fs::read_to_string(&config_path)
        .with_context(|| format!("无法读取配置文件: {config_path}"))?;

    serde_yaml::from_str(&config_text)
        .with_context(|| format!("配置文件 YAML 解析失败: {config_path}"))
}
