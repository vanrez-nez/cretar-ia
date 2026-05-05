export type RuntimePhase = "idle" | "recording" | "sending";

export type RuntimeStatus = "idle" | "loading" | "recording" | "sending" | "success" | "error" | "shutdown";

export interface AppRuntimeState {
  phase: RuntimePhase;
  status: RuntimeStatus;
  configReady: boolean;
  lastError: string | null;
}

export const DEFAULT_APP_RUNTIME_STATE: AppRuntimeState = {
  phase: "idle",
  status: "idle",
  configReady: false,
  lastError: null,
};
