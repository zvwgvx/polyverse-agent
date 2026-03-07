use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};

use anyhow::{Context, Result};
use serde::Deserialize;
const REGISTRY_RELATIVE_PATH: &str = "config/prompt_registry.json";

#[derive(Debug)]
pub struct PromptRegistry {
    root_dir: PathBuf,
    prompts: HashMap<String, String>,
    cache: RwLock<HashMap<String, String>>,
}

#[derive(Debug, Deserialize)]
struct PromptRegistryFile {
    prompts: HashMap<String, String>,
}

static REGISTRY: OnceLock<PromptRegistry> = OnceLock::new();

impl PromptRegistry {
    fn load_default() -> Result<Self> {
        let registry_path = find_registry_path()?;
        let raw = std::fs::read_to_string(&registry_path).with_context(|| {
            format!(
                "failed to read prompt registry file: {}",
                registry_path.display()
            )
        })?;
        let parsed: PromptRegistryFile =
            serde_json::from_str(&raw).context("failed to parse prompt registry json")?;

        let root_dir = registry_path
            .parent()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .context("failed to resolve project root from registry path")?;

        Ok(Self {
            root_dir,
            prompts: parsed.prompts,
            cache: RwLock::new(HashMap::new()),
        })
    }

    fn load_prompt(&self, id: &str) -> Result<String> {
        if let Ok(cache) = self.cache.read() {
            if let Some(content) = cache.get(id) {
                return Ok(content.clone());
            }
        }

        let rel_path = self
            .prompts
            .get(id)
            .with_context(|| format!("prompt id not found in registry: {}", id))?;

        let prompt_path = self.root_dir.join(rel_path);
        let content = std::fs::read_to_string(&prompt_path)
            .with_context(|| format!("failed to read prompt file: {}", prompt_path.display()))?;

        if let Ok(mut cache) = self.cache.write() {
            cache.insert(id.to_string(), content.clone());
        }

        Ok(content)
    }

    fn set_prompt(&self, id: &str, content: String) -> Result<()> {
        if !self.prompts.contains_key(id) {
            return Err(anyhow::anyhow!("prompt id not found in registry: {}", id));
        }

        let mut cache = self
            .cache
            .write()
            .map_err(|_| anyhow::anyhow!("prompt registry cache poisoned"))?;
        cache.insert(id.to_string(), content);
        Ok(())
    }
}

pub fn get_prompt(id: &str) -> Result<String> {
    let registry = registry()?;
    registry.load_prompt(id)
}

pub fn get_prompt_or(id: &str, fallback: &str) -> String {
    get_prompt(id).unwrap_or_else(|_| fallback.to_string())
}

pub fn render_prompt(id: &str, vars: &[(&str, &str)]) -> Result<String> {
    let content = get_prompt(id)?;
    Ok(apply_vars(&content, vars))
}

pub fn render_prompt_or(id: &str, vars: &[(&str, &str)], fallback: &str) -> String {
    let content = get_prompt(id).unwrap_or_else(|_| fallback.to_string());
    apply_vars(&content, vars)
}

pub fn set_prompt_content(id: &str, content: String) -> Result<()> {
    let registry = registry()?;
    registry.set_prompt(id, content)
}

fn registry() -> Result<&'static PromptRegistry> {
    if let Some(reg) = REGISTRY.get() {
        return Ok(reg);
    }

    let loaded = PromptRegistry::load_default()?;
    let _ = REGISTRY.set(loaded);

    REGISTRY
        .get()
        .context("failed to initialize prompt registry singleton")
}

fn find_registry_path() -> Result<PathBuf> {
    let mut dir = std::env::current_dir().context("failed to read current working directory")?;
    loop {
        let candidate = dir.join(REGISTRY_RELATIVE_PATH);
        if candidate.exists() {
            return Ok(candidate);
        }
        if !dir.pop() {
            break;
        }
    }

    Err(anyhow::anyhow!(
        "prompt registry not found (expected {})",
        REGISTRY_RELATIVE_PATH
    ))
}

fn apply_vars(template: &str, vars: &[(&str, &str)]) -> String {
    let mut output = template.to_string();
    for (key, value) in vars {
        let token = format!("{{{{{}}}}}", key);
        output = output.replace(&token, value);
    }
    output
}
