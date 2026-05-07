import Database from "@tauri-apps/plugin-sql";
import { defaultSettings, normalizeSettings, type SettingValue, type SettingsRecord } from "./schema";

const SETTINGS_DB_URL = "sqlite:cretar-ia.db";

type SettingsRow = {
  key: string;
  value: string;
};

let dbPromise: Promise<Database> | null = null;

export async function loadSettingsRows(): Promise<SettingsRecord> {
  const db = await settingsDb();
  const rows = await db.select<SettingsRow[]>("SELECT key, value FROM settings");
  const settings = defaultSettings();

  for (const row of rows) {
    if (!(row.key in settings)) {
      continue;
    }
    settings[row.key] = parseStoredValue(row.value);
  }

  const normalized = normalizeSettings(settings);
  await saveSettingsRows(normalized);
  return normalized;
}

export async function saveSettingsRows(settings: SettingsRecord): Promise<void> {
  const normalized = normalizeSettings(settings);
  const db = await settingsDb();
  const updatedAt = new Date().toISOString();

  for (const [key, value] of Object.entries(normalized)) {
    await db.execute(
      `INSERT INTO settings (key, value, updated_at) VALUES ($1, $2, $3)
       ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at`,
      [key, JSON.stringify(value), updatedAt]
    );
  }
}

async function settingsDb(): Promise<Database> {
  dbPromise ??= Database.load(SETTINGS_DB_URL);
  return dbPromise;
}

function parseStoredValue(value: string): SettingValue {
  try {
    const parsed = JSON.parse(value) as SettingValue;
    if (
      typeof parsed === "string" ||
      typeof parsed === "number" ||
      typeof parsed === "boolean" ||
      parsed === null
    ) {
      return parsed;
    }
  } catch {
    return value;
  }

  return value;
}
