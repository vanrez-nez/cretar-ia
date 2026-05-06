use crate::config::AppConfig;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager};

const CONTROL_FILE_NAME: &str = "settings-control.json";
const SETTINGS_WINDOW_LABEL: &str = "settings";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SettingsControlEndpoint {
    address: String,
    token: String,
    pid: u32,
}

#[derive(Debug, Serialize, Deserialize)]
struct SettingsControlMessage {
    token: String,
    event: SettingsControlMessageKind,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SettingsControlMessageKind {
    FocusSettings,
}

pub fn spawn_settings_control_server(app: AppHandle) -> Result<thread::JoinHandle<()>> {
    let listener = TcpListener::bind("127.0.0.1:0").context("binding settings control server")?;
    let address = listener
        .local_addr()
        .context("reading settings control server address")?
        .to_string();
    let token = settings_control_token();
    let endpoint = SettingsControlEndpoint {
        address: address.clone(),
        token: token.clone(),
        pid: std::process::id(),
    };
    write_endpoint(&endpoint)?;

    let handle = thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else {
                continue;
            };
            handle_stream(&mut stream, &token, &app);
        }
    });

    Ok(handle)
}

pub fn notify_settings_focus() -> Result<()> {
    notify_settings_control(SettingsControlMessageKind::FocusSettings)
}

pub fn cleanup_settings_control_endpoint() {
    let _ = fs::remove_file(endpoint_path());
}

fn notify_settings_control(event: SettingsControlMessageKind) -> Result<()> {
    let endpoint = read_endpoint()?;
    let message = SettingsControlMessage {
        token: endpoint.token,
        event,
    };
    let payload = serde_json::to_vec(&message).context("serializing settings control message")?;
    let mut stream = TcpStream::connect(&endpoint.address)
        .with_context(|| format!("connecting to settings control {}", endpoint.address))?;
    stream
        .write_all(&payload)
        .context("sending settings control message")?;
    Ok(())
}

fn handle_stream(stream: &mut TcpStream, token: &str, app: &AppHandle) {
    let mut payload = Vec::new();
    if let Err(err) = stream.read_to_end(&mut payload) {
        log::warn!("failed to read settings control message: {err}");
        return;
    }

    let message = match serde_json::from_slice::<SettingsControlMessage>(&payload) {
        Ok(message) => message,
        Err(err) => {
            log::warn!("invalid settings control message: {err}");
            return;
        }
    };

    if message.token != token {
        log::warn!("settings control message rejected: token mismatch");
        return;
    }

    match message.event {
        SettingsControlMessageKind::FocusSettings => focus_settings_window(app),
    }
}

fn focus_settings_window(app: &AppHandle) {
    let Some(window) = app.get_webview_window(SETTINGS_WINDOW_LABEL) else {
        log::warn!("settings focus requested but settings window was not found");
        return;
    };

    if let Err(err) = window.show() {
        log::warn!("failed to show settings window: {err}");
    }
    if let Err(err) = window.unminimize() {
        log::debug!("failed to unminimize settings window: {err}");
    }
    if let Err(err) = window.set_focus() {
        log::warn!("failed to focus settings window: {err}");
    }
}

fn write_endpoint(endpoint: &SettingsControlEndpoint) -> Result<()> {
    let path = endpoint_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    let payload = serde_json::to_vec(endpoint).context("serializing settings control endpoint")?;
    fs::write(&path, payload).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

fn read_endpoint() -> Result<SettingsControlEndpoint> {
    let path = endpoint_path();
    let payload = fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_slice(&payload).context("parsing settings control endpoint")
}

fn endpoint_path() -> PathBuf {
    AppConfig::base_dir().join(CONTROL_FILE_NAME)
}

fn settings_control_token() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!("{}-{nanos}", std::process::id())
}
