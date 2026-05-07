import { useSyncExternalStore } from "react";
import { isTauri } from "@tauri-apps/api/core";
import { tauriInvoke } from "../hooks/useTauriIPC";
import type { AppConfig } from "../lib/types";
import { DEFAULT_APP_CONFIG } from "../lib/types";
import type { AppRuntimeState } from "../lib/runtime";
import { DEFAULT_APP_RUNTIME_STATE } from "../lib/runtime";

type SettingsState = {
  config: AppConfig | null;
  appState: AppRuntimeState;
  isLoading: boolean;
  isSaving: boolean;
  isReady: boolean;
  error: string | null;
  fetchSettings: () => Promise<void>;
  updateSettings: (config: AppConfig) => Promise<void>;
};

type InternalSettingsState = Omit<SettingsState, "fetchSettings" | "updateSettings">;

const listeners = new Set<() => void>();
let nextSaveId = 1;
let latestSentSaveId = 0;
let state: InternalSettingsState = {
  config: null,
  appState: DEFAULT_APP_RUNTIME_STATE,
  isLoading: false,
  isSaving: false,
  isReady: false,
  error: null,
};

const actions = {
  fetchSettings: async () => {
    setState({
      isLoading: true,
      error: null,
      appState: {
        ...state.appState,
        status: "loading",
        phase: "idle",
        lastError: null,
      },
    });

    if (!isTauri()) {
      setState({
        config: DEFAULT_APP_CONFIG,
        isLoading: false,
        isReady: true,
        appState: {
          ...state.appState,
          configReady: true,
          status: "idle",
          lastError: null,
        },
      });
      return;
    }

    try {
      const config = await tauriInvoke<AppConfig>("load_config");
      setState({
        config,
        isLoading: false,
        isReady: true,
        appState: {
          ...state.appState,
          configReady: true,
          status: "success",
          lastError: null,
        },
        error: null,
      });
    } catch (error) {
      setState({
        config: DEFAULT_APP_CONFIG,
        isLoading: false,
        isReady: true,
        appState: {
          ...state.appState,
          status: "error",
          lastError: error instanceof Error ? error.message : String(error),
          configReady: false,
        },
        error: error instanceof Error ? error.message : String(error),
      });
    }
  },
  updateSettings: async (config: AppConfig) => {
    const saveId = nextSaveId++;
    latestSentSaveId = saveId;
    console.info("[settings] save queued", {
      saveId,
      fingerprint: configFingerprint(config),
    });

    setState({
      isSaving: true,
      error: null,
      appState: {
        ...state.appState,
        status: "sending",
        phase: "sending",
        lastError: null,
      },
    });

    try {
      if (!isTauri()) {
        console.info("[settings] save resolved", {
          saveId,
          stale: saveId < latestSentSaveId,
          fingerprint: configFingerprint(config),
        });
        setState({
          config,
          isReady: true,
          isSaving: false,
          appState: {
            ...state.appState,
            phase: "idle",
            status: "success",
            configReady: true,
            lastError: null,
          },
        });
        return;
      }

      console.info("[settings] save sent", {
        saveId,
        fingerprint: configFingerprint(config),
      });
      const savedConfig = await tauriInvoke<AppConfig>("save_config", { config, saveId });
      console.info("[settings] save resolved", {
        saveId,
        stale: saveId < latestSentSaveId,
        fingerprint: configFingerprint(savedConfig),
      });
      if (saveId < latestSentSaveId) {
        console.warn("[settings] stale save response ignored", {
          saveId,
          latestSentSaveId,
        });
        return;
      }
      setState({
        config: savedConfig,
        isSaving: false,
        isReady: true,
        appState: {
          ...state.appState,
          phase: "idle",
          status: "success",
          configReady: true,
          lastError: null,
        },
      });
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      console.error("[settings] save failed", {
        saveId,
        stale: saveId < latestSentSaveId,
        fingerprint: configFingerprint(config),
        error: message,
      });
      setState({
        isSaving: false,
        error: message,
        appState: {
          ...state.appState,
          status: "error",
          lastError: message,
          phase: "idle",
        },
      });
      throw error;
    }
  },
};

function setState(next: Partial<InternalSettingsState>): void {
  state = { ...state, ...next };
  for (const listener of listeners) {
    listener();
  }
}

export function useSettingsStore<T>(selector: (state: SettingsState) => T): T {
  return useSyncExternalStore(
    subscribe,
    () => selector(getState()),
    () => selector(getState())
  );
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function configFingerprint(config: AppConfig): Record<string, unknown> {
  return {
    language: config.ui?.language,
    mode: config.interaction.mode,
    shortcut: config.interaction.shortcut,
    inputDevice: config.audio.input_device,
    autoSwitchInput: config.audio.auto_switch_to_primary_device,
    startSound: config.audio_cues.start_sound,
    stopSound: config.audio_cues.stop_sound,
    errorSound: config.audio_cues.error_sound,
  };
}

function getState(): SettingsState {
  return {
    ...state,
    ...actions,
  };
}
