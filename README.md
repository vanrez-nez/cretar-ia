# cretar-ia

Minimal cross-platform dictation agent with:
- global shortcut control
- recording from mic
- OpenRouter transcription
- systray-style status updates (optional with `--features tray`)
- configurable start/stop/error cues
- home-directory config at `~/.cretar-ia/config.json`

## Config

`~/.cretar-ia/config.json` is created automatically on first run.

```json
{
  "provider": {
    "provider": "openrouter",
    "openrouter": {
      "api_key": "OPENROUTER_API_KEY",
      "model": "openai/whisper-1",
      "base_url": "https://openrouter.ai/api/v1",
      "endpoint": "audio/transcriptions",
      "prompt": null
    }
  },
  "interaction": {
    "mode": "push_to_talk",
    "shortcut": "ctrl+shift+space"
  },
  "audio": {
    "sample_rate": 16000,
    "channels": 1,
    "input_device": null,
    "max_duration_secs": 120,
    "recording_dir": "recordings"
  },
  "audio_cues": {
    "enabled": false,
    "start_sound": null,
    "stop_sound": null,
    "error_sound": null,
    "volume": 0.6
  },
  "output": {
    "mode": "clipboard_paste",
    "paste_delay_ms": 40
  },
  "tray": {
    "title": "Cretar IA",
    "icon": "idle",
    "tooltip": {
      "idle": "Cretar IA idle",
      "recording": "Cretar IA recording",
      "sending": "Cretar IA transcribing...",
      "success": "Cretar IA ready"
    },
    "refresh_ms": 500
  }
}
```

## Run (official Vite + Tauri layout)

Install dependencies:

```bash
npm install
```

Run the combined app (frontend + backend) in development:

```bash
npm run tauri dev
```

Important permission note (macOS):
- Clipboard actions do not trigger macOS privacy prompts in this app.
- Microphone permissions are tied to the app bundle runtime.
- If you only run the raw debug binary, permission prompts may not appear.
- Validate mic permission by running a built `.app` from `target/debug/bundle/macos` (or release) and granting access in **System Settings > Privacy & Security > Microphone**.

Run the tray app with only backend features (no settings window):

```bash
cd src-tauri
cargo run --features tray
```

Open the settings UI directly:

```bash
cd src-tauri
cargo run --features "tray settings-ui" -- --settings
```
