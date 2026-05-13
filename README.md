# Cretar IA

Cretar IA is a desktop dictation app for turning short voice recordings into text and pasting them into the app you are already using. It runs from the tray, listens for a global hotkey, records from your selected microphone, sends the audio to your configured transcription provider, and delivers the result through the clipboard/paste flow.

## What You Can Do With Cretar IA

- Dictate text into any app with a global hotkey.
- Use push-to-talk or toggle recording mode.
- Choose the microphone used for recording.
- See when the app is listening or transcribing in the tray and always-on-top widget.
- Transcribe with OpenRouter or OpenAI-compatible providers.
- Choose separate models for transcription and text cleanup.
- Clean up, rewrite, or format dictated text with reusable prompts.
- Paste results automatically, with clipboard fallback when paste automation is unavailable.
- Keep an optional local history of transcripts and retained recordings.
- Review, replay, delete, or export saved transcript history.
- Configure recording feedback sounds and app language.

## Getting Started

Open **Settings** from the tray and configure:

1. **Transcript model**: add/select an STT model provider. OpenRouter and OpenAI-compatible providers are supported.
2. **Transform model**: optionally add/select a formatting model for prompt-based cleanup or rewriting.
3. **Prompts**: choose or create reusable prompt templates for transcript transformation.
4. **Recording**: choose push-to-talk or toggle mode, set the hotkey, select the microphone, and configure sound cues.
5. **History**: enable retained history if you want transcripts and audio files kept locally for review/export.

The default hotkey is `Ctrl+Shift+Space`.

## Permissions

On macOS, grant the app:

- **Microphone** permission so it can record audio.
- **Accessibility** permission if you want automatic paste keystrokes to work.

Clipboard fallback can still place the transcript on the clipboard when paste automation is unavailable.

When testing microphone permissions, run the packaged `.app` bundle. macOS permission prompts are tied to the app bundle, and running the raw debug binary from a terminal may not show the expected prompt.

## Developer Build

Prerequisites:

- Node.js/npm
- Rust toolchain
- Tauri system prerequisites for your platform

Install dependencies:

```bash
npm install
```

Run the app in development:

```bash
npm run tauri dev
```

Run TypeScript checks:

```bash
npm run check
```

Run Rust tests with the app features enabled:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --features settings-ui,tray
```

Build a macOS DMG release:

```bash
npm run release
```
