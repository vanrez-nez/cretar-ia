use anyhow::{anyhow, Result};
use tauri_plugin_global_shortcut::Shortcut;

pub fn validate_shortcut(raw: &str) -> Result<()> {
    let value = raw.trim();
    if value.is_empty() {
        return Err(anyhow!("shortcut is empty"));
    }

    value
        .parse::<Shortcut>()
        .map(|_| ())
        .map_err(|err| anyhow!("invalid shortcut '{value}': {err}"))
}
