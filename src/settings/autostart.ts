import { isTauri } from "@tauri-apps/api/core";
import { disable, enable, isEnabled } from "@tauri-apps/plugin-autostart";

export async function loadLaunchAtStart(): Promise<boolean> {
  if (!isTauri()) {
    return false;
  }

  return isEnabled();
}

export async function setLaunchAtStart(enabled: boolean): Promise<boolean> {
  if (!isTauri()) {
    return enabled;
  }

  if (enabled) {
    await enable();
  } else {
    await disable();
  }

  return isEnabled();
}
