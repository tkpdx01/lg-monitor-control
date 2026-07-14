use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::monitor::{InputPreset, default_inputs};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Accent {
    #[default]
    Violet,
    Cyan,
    Rose,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MonitorPreferences {
    #[serde(default)]
    pub alias: String,
    #[serde(default)]
    pub inputs: Vec<InputPreset>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default = "current_schema_version")]
    pub schema_version: u32,
    #[serde(default = "enabled")]
    pub close_to_tray: bool,
    #[serde(default = "enabled")]
    pub global_hotkeys: bool,
    #[serde(default)]
    pub compact_cards: bool,
    #[serde(default)]
    pub accent: Accent,
    #[serde(default)]
    pub primary_monitor_id: Option<String>,
    #[serde(default)]
    pub monitors: BTreeMap<String, MonitorPreferences>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            schema_version: current_schema_version(),
            close_to_tray: true,
            global_hotkeys: true,
            compact_cards: false,
            accent: Accent::Violet,
            primary_monitor_id: None,
            monitors: BTreeMap::new(),
        }
    }
}

impl AppConfig {
    pub fn alias_for<'a>(&'a self, monitor_id: &str, fallback: &'a str) -> &'a str {
        self.monitors
            .get(monitor_id)
            .map(|preferences| preferences.alias.trim())
            .filter(|alias| !alias.is_empty())
            .unwrap_or(fallback)
    }

    pub fn inputs_for(&self, monitor_id: &str) -> Vec<InputPreset> {
        self.monitors
            .get(monitor_id)
            .map(|preferences| preferences.inputs.clone())
            .filter(|inputs| !inputs.is_empty())
            .unwrap_or_else(default_inputs)
    }

    pub fn preferences_mut(&mut self, monitor_id: &str) -> &mut MonitorPreferences {
        self.monitors
            .entry(monitor_id.to_owned())
            .or_insert_with(|| MonitorPreferences {
                alias: String::new(),
                inputs: default_inputs(),
            })
    }
}

pub struct ConfigStore {
    path: PathBuf,
    pub data: AppConfig,
    pub load_warning: Option<String>,
}

impl ConfigStore {
    pub fn load() -> Self {
        let path = config_path();
        match fs::read_to_string(&path) {
            Ok(contents) => match serde_json::from_str(&contents) {
                Ok(data) => Self {
                    path,
                    data,
                    load_warning: None,
                },
                Err(error) => Self {
                    path,
                    data: AppConfig::default(),
                    load_warning: Some(format!("Could not read settings: {error}")),
                },
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Self {
                path,
                data: AppConfig::default(),
                load_warning: None,
            },
            Err(error) => Self {
                path,
                data: AppConfig::default(),
                load_warning: Some(format!("Could not open settings: {error}")),
            },
        }
    }

    pub fn save(&self) -> Result<(), ConfigError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let contents = serde_json::to_string_pretty(&self.data)?;
        fs::write(&self.path, contents)?;
        Ok(())
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("Could not write settings: {0}")]
    Io(#[from] std::io::Error),
    #[error("Could not serialize settings: {0}")]
    Json(#[from] serde_json::Error),
}

fn config_path() -> PathBuf {
    ProjectDirs::from("dev", "tkpdx01", "LG Monitor Control")
        .map(|directories| directories.config_dir().join("settings.json"))
        .unwrap_or_else(|| PathBuf::from("lg-monitor-control.settings.json"))
}

const fn current_schema_version() -> u32 {
    1
}

const fn enabled() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_monitor_input_preferences_fall_back_to_defaults() {
        let mut config = AppConfig::default();
        config.monitors.insert(
            "monitor".into(),
            MonitorPreferences {
                alias: "Desk".into(),
                inputs: Vec::new(),
            },
        );

        assert_eq!(config.inputs_for("monitor"), default_inputs());
        assert_eq!(config.alias_for("monitor", "Fallback"), "Desk");
    }
}
