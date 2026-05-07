export type InteractionMode = "push_to_talk" | "toggle";

export type OutputMode = "clipboard_only" | "clipboard_paste";

export type AppLanguage = "system" | "en" | "es";

export type PermissionState =
  | "granted"
  | "denied"
  | "not_determined"
  | "restricted"
  | "unsupported"
  | "unknown";

export interface PermissionsStatus {
  microphone: PermissionState;
  accessibility: PermissionState;
}

export interface OpenRouterConfig {
  api_key: string;
  model: string;
  base_url: string;
  endpoint: string;
  prompt: string | null;
}

export interface ProviderConfig {
  provider: string;
  openrouter: OpenRouterConfig;
}

export interface AudioCaptureConfig {
  sample_rate: number;
  channels: number;
  input_device: string | null;
  auto_switch_to_primary_device: boolean;
  max_duration_secs: number;
  recording_dir: string;
}

export interface AudioCueConfig {
  enabled: boolean;
  start_sound: string | null;
  stop_sound: string | null;
  error_sound: string | null;
  volume: number;
}

export interface RecoveryStrategyConfig {
  retry_start_timeout: boolean;
  retry_stop_timeout: boolean;
  retry_processing_timeout: boolean;
  retry_queue_saturation: boolean;
}

export interface PipelineConfig {
  debounce_ms?: number | null;
  settle_timeout_ms?: number | null;
  hotkey_queue_capacity?: number | null;
  worker_queue_capacity?: number | null;
  queue_saturation_policy?: "retry" | "error_only" | null;
  recovery?: RecoveryStrategyConfig | null;
  max_recording_duration_secs?: number | null;
}

export interface OutputConfig {
  mode: OutputMode;
  paste_delay_ms: number;
  cleanup_recording_after_processing: boolean;
  processing_timeout_ms: number;
}

export interface TrayTooltipConfig {
  idle: string;
  recording: string;
  sending: string;
  success: string;
}

export interface TrayConfig {
  title: string;
  icon: string;
  tooltip: TrayTooltipConfig;
  refresh_ms: number;
}

export interface UiConfig {
  language: AppLanguage;
}

export interface AppConfig {
  ui: UiConfig;
  provider: ProviderConfig;
  interaction: {
    mode: InteractionMode;
    shortcut: string;
    repeat_debounce_ms: number;
  };
  pipeline: PipelineConfig;
  audio: AudioCaptureConfig;
  audio_cues: AudioCueConfig;
  output: OutputConfig;
  tray: TrayConfig;
}

export const DEFAULT_APP_CONFIG: AppConfig = {
  ui: {
    language: "system",
  },
  provider: {
    provider: "openrouter",
    openrouter: {
      api_key: "",
      model: "openai/whisper-1",
      base_url: "https://openrouter.ai/api/v1",
      endpoint: "audio/transcriptions",
      prompt: null,
    },
  },
  interaction: {
    mode: "push_to_talk",
    shortcut: "ctrl+shift+space",
    repeat_debounce_ms: 120,
  },
  pipeline: {
    debounce_ms: null,
    settle_timeout_ms: null,
    hotkey_queue_capacity: null,
    worker_queue_capacity: null,
    queue_saturation_policy: null,
    recovery: null,
    max_recording_duration_secs: null,
  },
  audio: {
    sample_rate: 0,
    channels: 0,
    input_device: null,
    auto_switch_to_primary_device: true,
    max_duration_secs: 120,
    recording_dir: "recordings",
  },
  audio_cues: {
    enabled: true,
    start_sound: "sounds/start.wav",
    stop_sound: "sounds/stop.wav",
    error_sound: "sounds/error_1.wav",
    volume: 0.6,
  },
  output: {
    mode: "clipboard_paste",
    paste_delay_ms: 40,
    cleanup_recording_after_processing: false,
    processing_timeout_ms: 30000,
  },
  tray: {
    title: "Cretar IA",
    icon: "idle",
    tooltip: {
      idle: "Cretar IA idle",
      recording: "Cretar IA recording",
      sending: "Cretar IA transcribing...",
      success: "Cretar IA ready",
    },
    refresh_ms: 3000,
  },
};
