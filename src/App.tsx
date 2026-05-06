import { useEffect, useState } from "react";
import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useSettingsStore } from "./stores/settingsStore";
import type { AppConfig } from "./lib/types";

function formatStatus({
  ready,
  isLoading,
  isSaving,
  error,
  runtimeStatus,
}: {
  ready: boolean;
  isLoading: boolean;
  isSaving: boolean;
  error: string | null;
  runtimeStatus: string;
}) {
  if (!isTauri()) {
    return "Running in browser mode: Tauri bridge is unavailable.";
  }
  if (error) {
    return `Error: ${error}`;
  }
  if (runtimeStatus === "loading" || isLoading) {
    return "Loading settings...";
  }
  if (runtimeStatus === "sending") {
    return "Saving settings...";
  }
  if (isSaving) {
    return "Saving settings...";
  }
  if (isLoading) {
    return "Loading settings...";
  }
  if (ready) {
    return "Settings loaded.";
  }
  return "Initializing...";
}

export default function App() {
  const fetchSettings = useSettingsStore((s) => s.fetchSettings);
  const config = useSettingsStore((s) => s.config);
  const isLoading = useSettingsStore((s) => s.isLoading);
  const isSaving = useSettingsStore((s) => s.isSaving);
  const isReady = useSettingsStore((s) => s.isReady);
  const appState = useSettingsStore((s) => s.appState);
  const saveSettings = useSettingsStore((s) => s.updateSettings);
  const loadError = useSettingsStore((s) => s.error);
  const [configText, setConfigText] = useState("");
  const [draftConfig, setDraftConfig] = useState<AppConfig | null>(null);
  const [message, setMessage] = useState("Loading settings...");
  const [editorError, setEditorError] = useState<string | null>(null);

  useEffect(() => {
    void fetchSettings();
  }, []);

  useEffect(() => {
    if (!config) {
      return;
    }
    setDraftConfig(config);
    setConfigText(JSON.stringify(config, null, 2));
  }, [config]);

  useEffect(() => {
    setMessage(
      formatStatus({
        ready: isReady,
        isLoading,
        isSaving,
        error: loadError,
        runtimeStatus: appState.status,
      })
    );
  }, [isReady, isLoading, isSaving, loadError, appState.status]);

  const saveConfig = async () => {
    if (!isTauri()) {
      setMessage("Running in browser mode: Tauri bridge is unavailable.");
      return;
    }

    try {
      const parsed = draftConfig ?? (JSON.parse(configText) as AppConfig);
      await saveSettings(parsed);
      setMessage("Saved.");
      setEditorError(null);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setEditorError(message);
      setMessage(`Error saving settings: ${message}`);
    }
  };

  const updateDraft = (updater: (config: AppConfig) => AppConfig) => {
    const source = draftConfig ?? config;
    if (!source) {
      return;
    }

    const next = updater(source);
    setDraftConfig(next);
    setConfigText(JSON.stringify(next, null, 2));
    setEditorError(null);
  };

  const updateFromJson = (value: string) => {
    setConfigText(value);
    try {
      setDraftConfig(JSON.parse(value) as AppConfig);
      setEditorError(null);
    } catch {
      setDraftConfig(null);
    }
  };

  const numberOrNull = (value: string): number | null => {
    if (value.trim() === "") {
      return null;
    }
    return Number(value);
  };

  const draft = draftConfig;

  const cancelSettings = async () => {
    try {
      const window = getCurrentWindow();
      await window.close();
    } catch {
      // no-op if called outside Tauri context
    }
  };

  return (
    <main className="container">
      <h1>Settings</h1>

      {draft ? (
        <section className="settings-grid">
          <section className="card settings-panel">
            <h2>Timeouts</h2>
            <label className="field">
              <span>OpenRouter processing timeout</span>
              <input
                type="number"
                min="0"
                max="600000"
                step="1000"
                value={draft.output.processing_timeout_ms}
                onChange={(event) =>
                  updateDraft((config) => ({
                    ...config,
                    output: {
                      ...config.output,
                      processing_timeout_ms: Number(event.currentTarget.value),
                    },
                  }))
                }
              />
              <small>Applies to transcription and text delivery. Use 0 to disable.</small>
            </label>

            <label className="field">
              <span>Pipeline settle timeout</span>
              <input
                type="number"
                min="100"
                max="120000"
                step="100"
                value={draft.pipeline.settle_timeout_ms ?? ""}
                placeholder="800"
                onChange={(event) =>
                  updateDraft((config) => ({
                    ...config,
                    pipeline: {
                      ...config.pipeline,
                      settle_timeout_ms: numberOrNull(event.currentTarget.value),
                    },
                  }))
                }
              />
              <small>Applies only to local start/stop worker transitions.</small>
            </label>

            <label className="toggle-field">
              <input
                type="checkbox"
                checked={draft.output.cleanup_recording_after_processing}
                onChange={(event) =>
                  updateDraft((config) => ({
                    ...config,
                    output: {
                      ...config.output,
                      cleanup_recording_after_processing: event.currentTarget.checked,
                    },
                  }))
                }
              />
              <span>Delete recording after successful processing</span>
            </label>

            <label className="toggle-field">
              <input
                type="checkbox"
                checked={draft.audio.auto_switch_to_primary_device}
                onChange={(event) =>
                  updateDraft((config) => ({
                    ...config,
                    audio: {
                      ...config.audio,
                      auto_switch_to_primary_device: event.currentTarget.checked,
                    },
                  }))
                }
              />
              <span>Auto-switch to primary input device when selected device is unavailable</span>
            </label>
          </section>

          <section className="card settings-panel">
            <h2>Audio cues</h2>
            <label className="toggle-field">
              <input
                type="checkbox"
                checked={draft.audio_cues.enabled}
                onChange={(event) =>
                  updateDraft((config) => ({
                    ...config,
                    audio_cues: {
                      ...config.audio_cues,
                      enabled: event.currentTarget.checked,
                    },
                  }))
                }
              />
              <span>Play recording feedback sounds</span>
            </label>

            <label className="field">
              <span>Volume</span>
              <input
                type="range"
                min="0"
                max="2"
                step="0.05"
                value={draft.audio_cues.volume}
                onChange={(event) =>
                  updateDraft((config) => ({
                    ...config,
                    audio_cues: {
                      ...config.audio_cues,
                      volume: Number(event.currentTarget.value),
                    },
                  }))
                }
              />
              <small>{Math.round(draft.audio_cues.volume * 100)}%</small>
            </label>

            <label className="field">
              <span>Start sound</span>
              <input
                type="text"
                value={draft.audio_cues.start_sound ?? ""}
                placeholder="sounds/start.wav"
                onChange={(event) =>
                  updateDraft((config) => ({
                    ...config,
                    audio_cues: {
                      ...config.audio_cues,
                      start_sound: event.currentTarget.value.trim() || null,
                    },
                  }))
                }
              />
            </label>

            <label className="field">
              <span>Stop sound</span>
              <input
                type="text"
                value={draft.audio_cues.stop_sound ?? ""}
                placeholder="sounds/stop.wav"
                onChange={(event) =>
                  updateDraft((config) => ({
                    ...config,
                    audio_cues: {
                      ...config.audio_cues,
                      stop_sound: event.currentTarget.value.trim() || null,
                    },
                  }))
                }
              />
            </label>

            <label className="field">
              <span>Error sound</span>
              <input
                type="text"
                value={draft.audio_cues.error_sound ?? ""}
                placeholder="sounds/error_1.wav"
                onChange={(event) =>
                  updateDraft((config) => ({
                    ...config,
                    audio_cues: {
                      ...config.audio_cues,
                      error_sound: event.currentTarget.value.trim() || null,
                    },
                  }))
                }
              />
            </label>
          </section>
        </section>
      ) : null}

      <section className="card">
        <h2>Advanced JSON</h2>
        <textarea
          id="config"
          value={configText}
          placeholder="Loading settings file..."
          onChange={(event) => updateFromJson(event.currentTarget.value)}
        />
      </section>

      <p className="message" id="message">
        {message}
      </p>
      {editorError ? <p className="error-message">{editorError}</p> : null}

      <div className="toolbar">
        <button
          className="btn btn-primary"
          type="button"
          disabled={!isReady || isLoading || isSaving || !isTauri()}
          onClick={saveConfig}
        >
          Save
        </button>
        <button className="btn btn-ghost" type="button" onClick={cancelSettings}>
          Cancel
        </button>
      </div>
    </main>
  );
}
