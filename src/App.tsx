import { useEffect, useState } from "react";
import { isTauri, invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

export default function App() {
  const [configText, setConfigText] = useState("");
  const [message, setMessage] = useState("Loading settings...");
  const [ready, setReady] = useState(false);

  useEffect(() => {
    const loadConfig = async () => {
      if (!isTauri()) {
        setMessage("Running in browser mode: Tauri bridge is unavailable.");
        setReady(false);
        return;
      }

      try {
        const cfg = (await invoke("load_config")) as unknown;
        setConfigText(JSON.stringify(cfg, null, 2));
        setMessage("Settings loaded.");
        setReady(true);
      } catch (error) {
        const errorMessage = error instanceof Error ? error.message : String(error);
        setMessage(`Error loading settings: ${errorMessage}`);
        setReady(false);
      }
    };

    void loadConfig();
  }, []);

  const saveConfig = async () => {
    if (!isTauri()) {
      setMessage("Running in browser mode: Tauri bridge is unavailable.");
      return;
    }

    try {
      const parsed = JSON.parse(configText);
      await invoke("save_config", { config: parsed });
      setMessage("Saved.");
      setReady(true);
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : String(error);
      setMessage(`Error saving settings: ${errorMessage}`);
      setReady(false);
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

      <div className="toolbar">
        <button className="btn btn-primary" type="button" disabled={!ready} onClick={saveConfig}>
          Save
        </button>
        <button className="btn btn-ghost" type="button" onClick={cancelSettings}>
          Cancel
        </button>
      </div>
    </main>
  );
}
