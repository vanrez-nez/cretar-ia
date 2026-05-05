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
  const [message, setMessage] = useState("Loading settings...");
  const [editorError, setEditorError] = useState<string | null>(null);

  useEffect(() => {
    void fetchSettings();
  }, []);

  useEffect(() => {
    if (!config) {
      return;
    }
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
      const parsed = JSON.parse(configText) as AppConfig;
      await saveSettings(parsed);
      setMessage("Saved.");
      setEditorError(null);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setEditorError(message);
      setMessage(`Error saving settings: ${message}`);
    }
  };

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

      <section className="card">
        <textarea
          id="config"
          value={configText}
          placeholder="Loading settings file..."
          onChange={(event) => setConfigText(event.currentTarget.value)}
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
