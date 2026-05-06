use crate::config::AppConfig;
use crate::runtime::compat::RuntimeControlEvent;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc::UnboundedSender;

const CONTROL_FILE_NAME: &str = "runtime-control.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RuntimeControlEndpoint {
    address: String,
    token: String,
    pid: u32,
}

#[derive(Debug, Serialize, Deserialize)]
struct RuntimeControlMessage {
    token: String,
    event: RuntimeControlMessageKind,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RuntimeControlMessageKind {
    ReloadRuntime,
    SwitchInputDevice,
    PauseHotkeysForSettings,
    ResumeHotkeysAfterSettings,
}

pub fn spawn_runtime_control_server(
    tx: UnboundedSender<RuntimeControlEvent>,
) -> Result<thread::JoinHandle<()>> {
    let listener = TcpListener::bind("127.0.0.1:0").context("binding runtime control server")?;
    let address = listener
        .local_addr()
        .context("reading runtime control server address")?
        .to_string();
    let token = runtime_control_token();
    let endpoint = RuntimeControlEndpoint {
        address: address.clone(),
        token: token.clone(),
        pid: std::process::id(),
    };
    write_endpoint(&endpoint)?;

    log::info!("runtime control server listening at {address}");
    let handle = thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else {
                continue;
            };
            handle_stream(&mut stream, &token, &tx);
        }
    });

    Ok(handle)
}

pub fn notify_runtime_reload() -> Result<()> {
    notify_runtime_control(RuntimeControlMessageKind::ReloadRuntime)
}

#[allow(dead_code)]
pub fn notify_input_device_switch() -> Result<()> {
    notify_runtime_control(RuntimeControlMessageKind::SwitchInputDevice)
}

pub fn notify_hotkeys_pause_for_settings() -> Result<()> {
    notify_runtime_control(RuntimeControlMessageKind::PauseHotkeysForSettings)
}

pub fn notify_hotkeys_resume_after_settings() -> Result<()> {
    notify_runtime_control(RuntimeControlMessageKind::ResumeHotkeysAfterSettings)
}

fn notify_runtime_control(event: RuntimeControlMessageKind) -> Result<()> {
    let endpoint = read_endpoint()?;
    let message = RuntimeControlMessage {
        token: endpoint.token,
        event,
    };
    let payload = serde_json::to_vec(&message).context("serializing runtime reload message")?;
    let mut stream = TcpStream::connect(&endpoint.address)
        .with_context(|| format!("connecting to runtime control {}", endpoint.address))?;
    stream
        .write_all(&payload)
        .context("sending runtime reload message")?;
    Ok(())
}

fn handle_stream(
    stream: &mut TcpStream,
    token: &str,
    tx: &UnboundedSender<RuntimeControlEvent>,
) {
    let mut payload = Vec::new();
    if let Err(err) = stream.read_to_end(&mut payload) {
        log::warn!("failed to read runtime control message: {err}");
        return;
    }

    let message = match serde_json::from_slice::<RuntimeControlMessage>(&payload) {
        Ok(message) => message,
        Err(err) => {
            log::warn!("invalid runtime control message: {err}");
            return;
        }
    };

    if message.token != token {
        log::warn!("runtime control message rejected: token mismatch");
        return;
    }

    match message.event {
        RuntimeControlMessageKind::ReloadRuntime => {
            log::info!("runtime reload requested by settings");
            let _ = tx.send(RuntimeControlEvent::ReloadRuntime);
        }
        RuntimeControlMessageKind::SwitchInputDevice => {
            log::info!("runtime input device switch requested");
            let _ = tx.send(RuntimeControlEvent::SwitchInputDevice);
        }
        RuntimeControlMessageKind::PauseHotkeysForSettings => {
            log::info!("runtime hotkey pause requested by settings");
            let _ = tx.send(RuntimeControlEvent::PauseHotkeysForSettings);
        }
        RuntimeControlMessageKind::ResumeHotkeysAfterSettings => {
            log::info!("runtime hotkey resume requested by settings");
            let _ = tx.send(RuntimeControlEvent::ResumeHotkeysAfterSettings);
        }
    }
}

fn write_endpoint(endpoint: &RuntimeControlEndpoint) -> Result<()> {
    let path = endpoint_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    let payload = serde_json::to_vec(endpoint).context("serializing runtime control endpoint")?;
    fs::write(&path, payload).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

fn read_endpoint() -> Result<RuntimeControlEndpoint> {
    let path = endpoint_path();
    let payload = fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_slice(&payload).context("parsing runtime control endpoint")
}

fn endpoint_path() -> PathBuf {
    AppConfig::base_dir().join(CONTROL_FILE_NAME)
}

fn runtime_control_token() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!("{}-{nanos}", std::process::id())
}
