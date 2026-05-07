import { useSyncExternalStore } from "react";
import { isTauri } from "@tauri-apps/api/core";
import { tauriInvoke } from "../hooks/useTauriIPC";
import type { AppRuntimeState } from "../lib/runtime";
import { DEFAULT_APP_RUNTIME_STATE } from "../lib/runtime";
import { logger } from "../lib/logger";
import { loadSettingsRows, saveSettingsRows } from "../settings/sql";
import {
  defaultSettings,
  settingFingerprint,
  type SettingsRecord,
} from "../settings/schema";

type SettingsState = {
  settings: SettingsRecord | null;
  appState: AppRuntimeState;
  isLoading: boolean;
  isSaving: boolean;
  isReady: boolean;
  error: string | null;
  fetchSettings: () => Promise<void>;
  updateSettings: (settings: SettingsRecord) => Promise<void>;
};

type InternalSettingsState = Omit<SettingsState, "fetchSettings" | "updateSettings">;

const listeners = new Set<() => void>();
let nextSaveId = 1;
let latestSentSaveId = 0;
let state: InternalSettingsState = {
  settings: null,
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
        settings: defaultSettings(),
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
      const settings = await loadSettingsRows();
      setState({
        settings,
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
      const message = error instanceof Error ? error.message : String(error);
      setState({
        settings: defaultSettings(),
        isLoading: false,
        isReady: true,
        appState: {
          ...state.appState,
          status: "error",
          lastError: message,
          configReady: false,
        },
        error: message,
      });
    }
  },
  updateSettings: async (settings: SettingsRecord) => {
    const saveId = nextSaveId++;
    latestSentSaveId = saveId;
    logger.info("[settings] save queued", {
      saveId,
      fingerprint: settingFingerprint(settings),
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
        setState({
          settings,
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

      await saveSettingsRows(settings);
      await tauriInvoke<void>("apply_settings", { saveId });
      logger.info("[settings] save resolved", {
        saveId,
        stale: saveId < latestSentSaveId,
        fingerprint: settingFingerprint(settings),
      });
      if (saveId < latestSentSaveId) {
        logger.warn("[settings] stale save response ignored", {
          saveId,
          latestSentSaveId,
        });
        return;
      }
      setState({
        settings,
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
      logger.error("[settings] save failed", {
        saveId,
        stale: saveId < latestSentSaveId,
        fingerprint: settingFingerprint(settings),
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

function getState(): SettingsState {
  return {
    ...state,
    ...actions,
  };
}
