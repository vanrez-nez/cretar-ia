use crate::config::{AppConfig, AppLanguage};
use serde_json::Value;
use std::sync::OnceLock;

const EN_LOCALE: &str = include_str!("../../src/i18n/locales/en.json");
const ES_LOCALE: &str = include_str!("../../src/i18n/locales/es.json");

static EN: OnceLock<Value> = OnceLock::new();
static ES: OnceLock<Value> = OnceLock::new();

pub fn t_config(config: &AppConfig, key: &str) -> String {
    let locale = locale_for_config(config);
    t_locale(locale, key).unwrap_or_else(|| key.to_string())
}

pub fn t_locale(locale: &str, key: &str) -> Option<String> {
    lookup(locale_value(locale), key)
        .or_else(|| lookup(locale_value("en"), key))
        .map(ToOwned::to_owned)
}

fn locale_for_config(config: &AppConfig) -> &'static str {
    match config.ui.language {
        AppLanguage::En => "en",
        AppLanguage::Es => "es",
        AppLanguage::System => system_locale(),
    }
}

fn system_locale() -> &'static str {
    let raw = std::env::var("LC_ALL")
        .ok()
        .filter(|value| !value.is_empty())
        .or_else(|| std::env::var("LC_MESSAGES").ok().filter(|value| !value.is_empty()))
        .or_else(|| std::env::var("LANG").ok().filter(|value| !value.is_empty()));

    let Some(raw) = raw else {
        return "en";
    };

    let language = raw
        .split(['_', '-', '.'])
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();

    match language.as_str() {
        "es" => "es",
        _ => "en",
    }
}

fn locale_value(locale: &str) -> &'static Value {
    match locale {
        "es" => ES.get_or_init(|| parse_locale(ES_LOCALE)),
        _ => EN.get_or_init(|| parse_locale(EN_LOCALE)),
    }
}

fn parse_locale(raw: &str) -> Value {
    serde_json::from_str(raw).unwrap_or(Value::Null)
}

fn lookup<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    let mut current = value;
    for part in key.split('.') {
        current = current.get(part)?;
    }
    current.as_str()
}
