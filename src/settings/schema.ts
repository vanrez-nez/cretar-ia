import Ajv from "ajv";
import schema from "./settings.schema.json";

export type SettingValue = string | number | boolean | null;
export type SettingsRecord = Record<string, SettingValue>;

const ajv = new Ajv({
  allErrors: true,
  strict: false,
  useDefaults: true,
});

const validate = ajv.compile(schema);

export function defaultSettings(): SettingsRecord {
  const settings: SettingsRecord = {};
  assertValidSettings(settings);
  return settings;
}

export function normalizeSettings(value: unknown): SettingsRecord {
  const settings = {
    ...defaultSettings(),
    ...(isRecord(value) ? value : {}),
  };
  assertValidSettings(settings);
  return settings;
}

export function assertValidSettings(value: unknown): asserts value is SettingsRecord {
  if (!validate(value)) {
    throw new Error(
      `invalid settings: ${ajv.errorsText(validate.errors, { separator: "; " })}`
    );
  }
}

export function settingFingerprint(settings: SettingsRecord): Record<string, SettingValue> {
  return {
    "system.language": settings["system.language"],
    "recording.mode": settings["recording.mode"],
    "recording.hotkey": settings["recording.hotkey"],
    "recording.microphone.input_device": settings["recording.microphone.input_device"],
    "recording.microphone.auto_switch_to_primary": settings["recording.microphone.auto_switch_to_primary"],
    "recording.sounds.start": settings["recording.sounds.start"],
    "recording.sounds.stop": settings["recording.sounds.stop"],
    "recording.sounds.error": settings["recording.sounds.error"],
  };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
