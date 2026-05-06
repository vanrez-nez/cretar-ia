use anyhow::{Context, Result};
use serde_json::{Map, Value};

pub fn migrate(raw: &str) -> Result<Value> {
    let mut value: Value = serde_json::from_str(raw).with_context(|| "invalid config json")?;

    let object = value
        .as_object_mut()
        .context("config root must be a JSON object")?;

    if !object.contains_key("pipeline") {
        object.insert("pipeline".to_string(), Value::Object(Map::new()));
    }

    let legacy_debounce_ms = object
        .get("interaction")
        .and_then(Value::as_object)
        .and_then(|interaction| interaction.get("repeat_debounce_ms"))
        .cloned();
    let legacy_hotkey_queue_capacity = object
        .get("interaction")
        .and_then(Value::as_object)
        .and_then(|interaction| interaction.get("hotkey_queue_capacity"))
        .cloned();
    let legacy_worker_queue_capacity = object
        .get("interaction")
        .and_then(Value::as_object)
        .and_then(|interaction| interaction.get("worker_queue_capacity"))
        .cloned();
    let legacy_max_recording_duration_secs = object
        .get("audio")
        .and_then(Value::as_object)
        .and_then(|audio| audio.get("max_duration_secs"))
        .cloned();

    let Some(pipeline) = object.get_mut("pipeline").and_then(Value::as_object_mut) else {
        return Ok(value);
    };

    if !pipeline.contains_key("debounce_ms") {
        if let Some(value) = legacy_debounce_ms {
            pipeline.insert("debounce_ms".to_string(), value);
        }
    }

    if !pipeline.contains_key("hotkey_queue_capacity") {
        if let Some(value) = legacy_hotkey_queue_capacity {
            pipeline.insert("hotkey_queue_capacity".to_string(), value);
        }
    }

    if !pipeline.contains_key("worker_queue_capacity") {
        if let Some(value) = legacy_worker_queue_capacity {
            pipeline.insert("worker_queue_capacity".to_string(), value);
        }
    }

    if !pipeline.contains_key("max_recording_duration_secs") {
        if let Some(value) = legacy_max_recording_duration_secs {
            pipeline.insert("max_recording_duration_secs".to_string(), value);
        }
    }

    Ok(value)
}
