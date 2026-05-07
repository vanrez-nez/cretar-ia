import { isTauri } from "@tauri-apps/api/core";
import { attachConsole, debug, error, info, trace, warn } from "@tauri-apps/plugin-log";

let consoleAttached = false;

export async function attachTauriLogger(): Promise<void> {
  if (!isTauri() || consoleAttached) {
    return;
  }

  consoleAttached = true;
  try {
    await attachConsole();
  } catch (err) {
    console.warn("failed to attach Tauri logger", err);
  }
}

export const logger = {
  trace: (message: string, payload?: unknown) => writeLog(trace, "trace", message, payload),
  debug: (message: string, payload?: unknown) => writeLog(debug, "debug", message, payload),
  info: (message: string, payload?: unknown) => writeLog(info, "info", message, payload),
  warn: (message: string, payload?: unknown) => writeLog(warn, "warn", message, payload),
  error: (message: string, payload?: unknown) => writeLog(error, "error", message, payload),
};

function writeLog(
  writer: (message: string) => Promise<void>,
  level: "trace" | "debug" | "info" | "warn" | "error",
  message: string,
  payload?: unknown
): void {
  const text = payload === undefined ? message : `${message} ${safeStringify(payload)}`;

  if (!isTauri()) {
    console[level](text);
    return;
  }

  void writer(text).catch((err) => {
    console[level](text);
    console.warn("failed to write Tauri log", err);
  });
}

function safeStringify(value: unknown): string {
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}
