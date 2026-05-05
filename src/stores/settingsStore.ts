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

      await tauriInvoke<void>("save_config", { config });
      setState({
        config,
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
