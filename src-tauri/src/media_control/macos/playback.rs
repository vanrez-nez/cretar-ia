use std::ffi::{c_char, c_void, CString};
use std::io::Write;
use std::process::{Command, Stdio};
use std::ptr;
use std::thread;
use std::time::Duration;

const RTLD_NOW: i32 = 2;
const MR_COMMAND_PLAY: i32 = 0;
const MR_COMMAND_PAUSE: i32 = 1;

type MrMediaRemoteSendCommand = unsafe extern "C" fn(i32, *const c_void) -> u8;

extern "C" {
    fn dlopen(filename: *const c_char, flag: i32) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dlclose(handle: *mut c_void) -> i32;
}

const NOW_PLAYING_JXA_SCRIPT: &str = r#"
function run() {
  const MediaRemote = $.NSBundle.bundleWithPath(
    "/System/Library/PrivateFrameworks/MediaRemote.framework/",
  );
  MediaRemote.load;

  const MRNowPlayingRequest = $.NSClassFromString("MRNowPlayingRequest");
  const client = MRNowPlayingRequest.localNowPlayingPlayerPath.client;
  const clientConverted = {
    bundleIdentifier: client.bundleIdentifier.js,
    parentApplicationBundleIdentifier:
      client.parentApplicationBundleIdentifier.js,
  };

  const infoDict = MRNowPlayingRequest.localNowPlayingItem.nowPlayingInfo;
  const infoConverted = {};
  for (const key in infoDict.js) {
    const value = infoDict.valueForKey(key).js;
    if (typeof value !== "object") {
      infoConverted[key] = value;
    } else if (value && typeof value.getTime === "function") {
      try {
        infoConverted[key] = value.getTime();
      } catch (e) {
        infoConverted[key] = value.toString();
      }
    } else {
      infoConverted[key] = value.toString();
    }
  }

  return JSON.stringify({
    isPlaying: MRNowPlayingRequest.localIsPlaying,
    client: clientConverted,
    info: infoConverted,
  });
}
"#;

pub fn pause_if_playing() -> bool {
    log::debug!("media pause: checking macOS now-playing state");
    if !is_playing().unwrap_or(false) {
        log::info!("media pause: no active playback detected");
        return false;
    }

    log::info!("media pause: active playback detected; sending pause");
    let paused = if send_media_remote_command("pause", MR_COMMAND_PAUSE) {
        wait_for_playback_state(false)
    } else {
        false
    };
    let paused = if paused {
        true
    } else if toggle_playback() {
        wait_for_playback_state(false)
    } else {
        false
    };

    if paused {
        log::info!("media pause: paused playback for recording");
    } else {
        log::warn!("media pause: failed to pause playback");
    }
    paused
}

pub fn resume_if_paused_by_us() -> bool {
    log::debug!("media resume: checking macOS now-playing state");
    if is_playing().unwrap_or(false) {
        log::info!("media resume: skipped because playback is already active");
        return false;
    }

    log::info!("media resume: resuming playback paused by this app");
    let resumed = if send_media_remote_command("play", MR_COMMAND_PLAY) {
        wait_for_playback_state(true)
    } else {
        false
    };
    let resumed = if resumed {
        true
    } else if toggle_playback() {
        wait_for_playback_state(true)
    } else {
        false
    };

    if resumed {
        log::info!("media resume: resumed playback");
    } else {
        log::warn!("media resume: failed to resume playback");
    }
    resumed
}

fn is_playing() -> Option<bool> {
    let mut child = Command::new("/usr/bin/osascript")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .arg("-l")
        .arg("JavaScript")
        .spawn()
        .ok()?;

    child
        .stdin
        .as_mut()?
        .write_all(NOW_PLAYING_JXA_SCRIPT.as_bytes())
        .ok()?;

    let output = child.wait_with_output().ok()?;
    if !output.status.success() {
        if log::log_enabled!(log::Level::Debug) {
            log::warn!(
                "media pause: macOS now-playing query failed status={:?} stdout={:?} stderr={:?}",
                output.status,
                truncate(&String::from_utf8_lossy(&output.stdout)),
                truncate(&String::from_utf8_lossy(&output.stderr))
            );
        }
        return None;
    }

    let raw: serde_json::Value = match serde_json::from_slice(&output.stdout) {
        Ok(value) => value,
        Err(err) => {
            if log::log_enabled!(log::Level::Debug) {
                log::warn!(
                    "media pause: macOS now-playing JSON parse failed error={err:?} stdout={:?} stderr={:?}",
                    truncate(&String::from_utf8_lossy(&output.stdout)),
                    truncate(&String::from_utf8_lossy(&output.stderr))
                );
            }
            return None;
        }
    };

    raw.get("isPlaying").and_then(|value| value.as_bool())
}

fn toggle_playback() -> bool {
    let output = Command::new("/usr/bin/osascript")
        .arg("-e")
        .arg("tell application \"System Events\" to key code 100")
        .output();

    match output {
        Ok(output) if output.status.success() => true,
        Ok(output) => {
            log::warn!(
                "media pause: macOS media-key toggle failed status={:?} stdout={:?} stderr={:?}",
                output.status,
                String::from_utf8_lossy(&output.stdout).trim(),
                String::from_utf8_lossy(&output.stderr).trim()
            );
            false
        }
        Err(err) => {
            log::warn!("media pause: failed to run macOS media-key toggle: {err}");
            false
        }
    }
}

fn wait_for_playback_state(expected: bool) -> bool {
    for _ in 0..5 {
        thread::sleep(Duration::from_millis(120));
        if is_playing() == Some(expected) {
            return true;
        }
    }
    false
}

fn send_media_remote_command(label: &str, command: i32) -> bool {
    let framework =
        CString::new("/System/Library/PrivateFrameworks/MediaRemote.framework/MediaRemote")
            .expect("static path has no nul");
    let symbol = CString::new("MRMediaRemoteSendCommand").expect("static symbol has no nul");

    unsafe {
        let handle = dlopen(framework.as_ptr(), RTLD_NOW);
        if handle.is_null() {
            log::debug!("media pause: MediaRemote dlopen failed for command={label}");
            return false;
        }

        let function = dlsym(handle, symbol.as_ptr());
        if function.is_null() {
            log::debug!("media pause: MediaRemote symbol missing for command={label}");
            let _ = dlclose(handle);
            return false;
        }

        let send_command: MrMediaRemoteSendCommand = std::mem::transmute(function);
        let ok = send_command(command, ptr::null()) != 0;
        let _ = dlclose(handle);
        log::debug!("media pause: MediaRemote command={label} accepted={ok}");
        ok
    }
}

fn truncate(value: &str) -> String {
    value.trim().chars().take(400).collect()
}
