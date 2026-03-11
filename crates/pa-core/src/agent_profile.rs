use std::path::PathBuf;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

const PROFILE_RELATIVE_PATH: &str = "config/agent_profile.toml";
const PROFILE_SAMPLE_RELATIVE_PATH: &str = "config/agent_profile.toml.sample";
const DEFAULT_DATA_DIR: &str = "data/polyverse-agent";

static AGENT_PROFILE: OnceLock<AgentProfile> = OnceLock::new();

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentProfile {
    #[serde(default = "default_agent_id")]
    pub agent_id: String,
    #[serde(default = "default_display_name")]
    pub display_name: String,
    #[serde(default)]
    pub graph_self_id: String,
    #[serde(default)]
    pub memory_db_path: String,
    #[serde(default)]
    pub graph_db_path: String,
    #[serde(default)]
    pub episodic_db_path: String,
    #[serde(default = "default_agent_timezone_label")]
    pub agent_timezone_label: String,
    #[serde(default = "default_agent_timezone_offset_hours")]
    pub agent_timezone_offset_hours: i32,
    #[serde(default = "default_user_timezone_label")]
    pub user_timezone_label: String,
    #[serde(default = "default_user_timezone_offset_hours")]
    pub user_timezone_offset_hours: i32,
}

impl Default for AgentProfile {
    fn default() -> Self {
        let mut profile = Self {
            agent_id: default_agent_id(),
            display_name: default_display_name(),
            graph_self_id: String::new(),
            memory_db_path: String::new(),
            graph_db_path: String::new(),
            episodic_db_path: String::new(),
            agent_timezone_label: default_agent_timezone_label(),
            agent_timezone_offset_hours: default_agent_timezone_offset_hours(),
            user_timezone_label: default_user_timezone_label(),
            user_timezone_offset_hours: default_user_timezone_offset_hours(),
        };
        profile.normalize();
        profile
    }
}

impl AgentProfile {
    fn normalize(&mut self) {
        self.agent_id = self.agent_id.trim().to_string();
        if self.agent_id.is_empty() {
            self.agent_id = default_agent_id();
        }

        self.display_name = self.display_name.trim().to_string();
        if self.display_name.is_empty() {
            self.display_name = self.agent_id.clone();
        }

        self.graph_self_id = self.graph_self_id.trim().to_string();
        if self.graph_self_id.is_empty() {
            self.graph_self_id = format!("person:{}", self.agent_id);
        }

        self.memory_db_path = self.memory_db_path.trim().to_string();
        if self.memory_db_path.is_empty() {
            self.memory_db_path = format!("{}/memory.db", DEFAULT_DATA_DIR);
        }

        self.graph_db_path = self.graph_db_path.trim().to_string();
        if self.graph_db_path.is_empty() {
            self.graph_db_path = format!("{}/graph", DEFAULT_DATA_DIR);
        }

        self.episodic_db_path = self.episodic_db_path.trim().to_string();
        if self.episodic_db_path.is_empty() {
            self.episodic_db_path = format!("{}/lancedb", DEFAULT_DATA_DIR);
        }

        self.agent_timezone_label = self.agent_timezone_label.trim().to_string();
        if self.agent_timezone_label.is_empty() {
            self.agent_timezone_label = default_agent_timezone_label();
        }

        self.user_timezone_label = self.user_timezone_label.trim().to_string();
        if self.user_timezone_label.is_empty() {
            self.user_timezone_label = default_user_timezone_label();
        }
    }

    fn apply_env_overrides(&mut self) {
        if let Ok(value) = std::env::var("PA_AGENT_ID") {
            self.agent_id = value;
        }
        if let Ok(value) = std::env::var("PA_AGENT_DISPLAY_NAME") {
            self.display_name = value;
        }
        if let Ok(value) = std::env::var("PA_GRAPH_SELF_ID") {
            self.graph_self_id = value;
        }
        if let Ok(value) = std::env::var("MEMORY_DB_PATH") {
            self.memory_db_path = value;
        }
        if let Ok(value) = std::env::var("GRAPH_DB_PATH") {
            self.graph_db_path = value;
        }
        if let Ok(value) = std::env::var("LANCE_DB_PATH") {
            self.episodic_db_path = value;
        }
        if let Ok(value) = std::env::var("AGENT_TIMEZONE_LABEL") {
            self.agent_timezone_label = value;
        }
        if let Ok(value) = std::env::var("AGENT_TIMEZONE_OFFSET_HOURS") {
            if let Ok(parsed) = value.parse::<i32>() {
                self.agent_timezone_offset_hours = parsed;
            }
        }
        if let Ok(value) = std::env::var("USER_TIMEZONE_LABEL") {
            self.user_timezone_label = value;
        }
        if let Ok(value) = std::env::var("USER_TIMEZONE_OFFSET_HOURS") {
            if let Ok(parsed) = value.parse::<i32>() {
                self.user_timezone_offset_hours = parsed;
            }
        }

        self.normalize();
    }
}

pub fn get_agent_profile() -> &'static AgentProfile {
    AGENT_PROFILE.get_or_init(|| match load_agent_profile() {
        Ok(profile) => profile,
        Err(error) => {
            tracing::warn!(error = %error, "Failed to load agent profile. Falling back to defaults.");
            AgentProfile::default()
        }
    })
}

fn load_agent_profile() -> anyhow::Result<AgentProfile> {
    let mut profile = if let Some(path) = find_profile_path()? {
        let raw = std::fs::read_to_string(&path)
            .map_err(|err| anyhow::anyhow!("failed to read agent profile {}: {}", path.display(), err))?;
        let mut parsed: AgentProfile = toml::from_str(&raw)
            .map_err(|err| anyhow::anyhow!("failed to parse agent profile {}: {}", path.display(), err))?;
        parsed.normalize();
        parsed
    } else {
        AgentProfile::default()
    };

    profile.apply_env_overrides();
    Ok(profile)
}

fn find_profile_path() -> anyhow::Result<Option<PathBuf>> {
    if let Ok(explicit) = std::env::var("PA_AGENT_PROFILE") {
        let path = PathBuf::from(explicit);
        if path.exists() {
            return Ok(Some(path));
        }
        return Err(anyhow::anyhow!(
            "PA_AGENT_PROFILE points to a missing file: {}",
            path.display()
        ));
    }

    let mut dir = std::env::current_dir()
        .map_err(|err| anyhow::anyhow!("failed to read current working directory: {}", err))?;
    loop {
        let primary = dir.join(PROFILE_RELATIVE_PATH);
        if primary.exists() {
            return Ok(Some(primary));
        }

        let sample = dir.join(PROFILE_SAMPLE_RELATIVE_PATH);
        if sample.exists() {
            return Ok(Some(sample));
        }

        if !dir.pop() {
            break;
        }
    }

    Ok(None)
}

pub fn sanitize_component(value: &str) -> String {
    value.replace(['`', '"', '\''], "")
}

pub fn default_agent_id() -> String {
    "agent".to_string()
}

pub fn default_display_name() -> String {
    "Agent".to_string()
}

fn default_agent_timezone_label() -> String {
    "GMT+8".to_string()
}

fn default_agent_timezone_offset_hours() -> i32 {
    8
}

fn default_user_timezone_label() -> String {
    "GMT+7".to_string()
}

fn default_user_timezone_offset_hours() -> i32 {
    7
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_profile_defaults_are_generic() {
        let profile = AgentProfile::default();
        assert_eq!(profile.agent_id, "agent");
        assert_eq!(profile.display_name, "Agent");
        assert_eq!(profile.graph_self_id, "person:agent");
        assert_eq!(profile.memory_db_path, "data/polyverse-agent/memory.db");
        assert_eq!(profile.graph_db_path, "data/polyverse-agent/graph");
        assert_eq!(profile.episodic_db_path, "data/polyverse-agent/lancedb");
    }

    #[test]
    fn test_sanitize_component() {
        assert_eq!(sanitize_component("ab`c\"d'e"), "abcde");
    }
}
