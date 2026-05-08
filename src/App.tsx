import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { isTauri } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { useTranslation } from "react-i18next";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Separator } from "@/components/ui/separator";
import { Switch } from "@/components/ui/switch";
import { Tabs, TabsContent } from "@/components/ui/tabs";
import { SettingsNavigation } from "@/components/sidebar-settings";
import { HotkeyCapture } from "@/components/hotkey";
import { JsonTextarea } from "@/components/json-textarea";
import { SettingsBadge } from "@/components/settings-badge";
import { tauriInvoke, useTauriEvent } from "@/hooks/useTauriIPC";
import i18n, { resolveAppLocale } from "@/i18n";
import { logger } from "@/lib/logger";
import { setLaunchAtStart } from "@/settings/autostart";
import type { SettingsRecord, SettingValue } from "@/settings/schema";
import { useSettingsStore } from "./stores/settingsStore";
import type { AppLanguage, InteractionMode, PermissionState, PermissionsStatus } from "./lib/types";
import { Check, Play, Plus, RefreshCw, ShieldCheck, ShieldX, SquarePen, Trash2, TriangleAlert } from "lucide-react";

const AUTOSAVE_DELAY_MS = 500;
const APP_VERSION = "0.1.0";
const SYSTEM_DEFAULT_INPUT_DEVICE = "__system_default__";

type SettingsChangedEvent = {
  source: string;
  keys: string[];
};

type SoundOption = {
  id: string;
  label: string;
  file: string;
};

type SoundSlot = "start" | "stop" | "error";

export default function App() {
  const { t } = useTranslation();
  const fetchSettings = useSettingsStore((s) => s.fetchSettings);
  const settings = useSettingsStore((s) => s.settings);
  const isReady = useSettingsStore((s) => s.isReady);
  const saveSettings = useSettingsStore((s) => s.updateSettings);
  const [draftSettings, setDraftSettings] = useState<SettingsRecord | null>(null);
  const [permissions, setPermissions] = useState<PermissionsStatus | null>(null);
  const [isRefreshingPermissions, setIsRefreshingPermissions] = useState(false);
  const [permissionError, setPermissionError] = useState<string | null>(null);
  const [isUpdatingAutostart, setIsUpdatingAutostart] = useState(false);
  const [inputDevices, setInputDevices] = useState<string[]>([]);
  const [soundOptions, setSoundOptions] = useState<SoundOption[]>([]);
  const [previewingSoundKey, setPreviewingSoundKey] = useState<string | null>(null);
  const hasHydrated = useRef(false);
  const skipNextAutosave = useRef(true);

  const checkPermissions = useCallback(async () => {
    if (!isTauri()) {
      setPermissions({
        microphone: "unsupported",
        accessibility: "unsupported",
      });
      return;
    }

    setPermissions(await tauriInvoke<PermissionsStatus>("check_permissions"));
  }, []);

  const refreshPermissions = useCallback(async () => {
    setIsRefreshingPermissions(true);
    setPermissionError(null);

    try {
      if (!isTauri()) {
        setPermissions({
          microphone: "unsupported",
          accessibility: "unsupported",
        });
        return;
      }

      const current = await tauriInvoke<PermissionsStatus>("check_permissions");

      if (current.microphone !== "granted") {
        await tauriInvoke<PermissionState>("request_microphone_permission");
      }

      if (current.accessibility !== "granted") {
        await tauriInvoke<PermissionState>("request_accessibility_permission");
      }

      setPermissions(await tauriInvoke<PermissionsStatus>("check_permissions"));
    } catch (error) {
      setPermissionError(error instanceof Error ? error.message : String(error));
    } finally {
      setIsRefreshingPermissions(false);
    }
  }, []);

  useEffect(() => {
    void fetchSettings();
  }, []);

  useEffect(() => {
    void checkPermissions().catch((error) => {
      setPermissionError(error instanceof Error ? error.message : String(error));
    });
  }, [checkPermissions]);

  useEffect(() => {
    if (!isTauri()) {
      setInputDevices([]);
      return;
    }

    void tauriInvoke<string[]>("list_input_devices")
      .then(setInputDevices)
      .catch((error) => {
        logger.error("failed to list input devices", {
          error: error instanceof Error ? error.message : String(error),
        });
      });
  }, []);

  useEffect(() => {
    if (!isTauri()) {
      setSoundOptions([]);
      return;
    }

    void tauriInvoke<SoundOption[]>("list_sound_options")
      .then(setSoundOptions)
      .catch((error) => {
        logger.error("failed to list sound options", {
          error: error instanceof Error ? error.message : String(error),
        });
      });
  }, []);

  useTauriEvent<SettingsChangedEvent>("settings:changed", (event) => {
    const shouldFetch = event.keys.some((key) =>
      key === "recording.microphone.input_device" || key === "models.formatting.enabled"
    );
    if (!shouldFetch) {
      return;
    }
    hasHydrated.current = false;
    skipNextAutosave.current = true;
    void fetchSettings();
  });

  useEffect(() => {
    if (!settings) {
      return;
    }
    if (hasHydrated.current) {
      return;
    }
    setDraftSettings(settings);
    void i18n.changeLanguage(resolveAppLocale(settings["system.language"] as AppLanguage));
    skipNextAutosave.current = true;
    hasHydrated.current = true;
  }, [settings]);

  useEffect(() => {
    if (!draftSettings || !hasHydrated.current) {
      return;
    }
    void i18n.changeLanguage(resolveAppLocale(draftSettings["system.language"] as AppLanguage));
    if (skipNextAutosave.current) {
      skipNextAutosave.current = false;
      return;
    }

    const timeout = window.setTimeout(() => {
      void saveSettings(draftSettings).catch((error) => {
        logger.error("failed to autosave settings", {
          error: error instanceof Error ? error.message : String(error),
        });
      });
    }, AUTOSAVE_DELAY_MS);

    return () => window.clearTimeout(timeout);
  }, [draftSettings, saveSettings]);

  const updateDraft = (key: string, value: SettingValue) => {
    setDraftSettings((current) => {
      if (!current) {
        return current;
      }
      return {
        ...current,
        [key]: value,
      };
    });
  };

  const updateLaunchAtStart = async (checked: boolean) => {
    setIsUpdatingAutostart(true);
    try {
      const enabled = await setLaunchAtStart(checked);
      updateDraft("system.launch_at_start", enabled);
    } catch (error) {
      logger.error("failed to update autostart", {
        error: error instanceof Error ? error.message : String(error),
      });
    } finally {
      setIsUpdatingAutostart(false);
    }
  };

  const previewSound = async (key: string, path: string | null) => {
    if (!path || !isTauri()) {
      return;
    }

    setPreviewingSoundKey(key);
    try {
      await tauriInvoke<void>("preview_sound", { path });
    } catch (error) {
      logger.error("failed to preview sound", {
        key,
        error: error instanceof Error ? error.message : String(error),
      });
    } finally {
      setPreviewingSoundKey((current) => (current === key ? null : current));
    }
  };

  const draft = draftSettings;

  return (
    <main className="h-screen select-none overflow-hidden bg-background text-foreground">
      <div className="h-full w-full overflow-x-hidden">
        {!draft ? (
          <Card className="m-5">
            <CardHeader>
              <CardTitle>{t("loading.title")}</CardTitle>
              <CardDescription>
                {isTauri() ? t("loading.descriptionTauri") : t("loading.descriptionPreview")}
              </CardDescription>
            </CardHeader>
          </Card>
        ) : (
          <Tabs defaultValue="system" orientation="vertical" className="grid h-full w-full grid-cols-[max-content_minmax(0,1fr)] gap-0 overflow-hidden">
            <SettingsNavigation />

            <TabsContent value="system" className="h-screen overflow-x-hidden overflow-y-auto overscroll-contain p-5">
              <SystemPane
                draft={draft}
                updateDraft={updateDraft}
                isUpdatingAutostart={isUpdatingAutostart}
                updateLaunchAtStart={updateLaunchAtStart}
              />
            </TabsContent>

            <TabsContent value="recording" className="h-screen overflow-x-hidden overflow-y-auto overscroll-contain p-5">
              <RecordingPane
                draft={draft}
                updateDraft={updateDraft}
                disabled={!isReady}
                inputDevices={inputDevices}
                soundOptions={soundOptions}
                previewingSoundKey={previewingSoundKey}
                previewSound={previewSound}
                permissions={permissions}
                isRefreshingPermissions={isRefreshingPermissions}
                permissionError={permissionError}
                refreshPermissions={refreshPermissions}
              />
            </TabsContent>

            <TabsContent value="transcripts" className="h-screen overflow-x-hidden overflow-y-auto overscroll-contain p-5">
              <ModelRolePane role="stt" draft={draft} updateDraft={updateDraft} />
            </TabsContent>

            <TabsContent value="transforms" className="h-screen overflow-x-hidden overflow-y-auto overscroll-contain p-5">
              <ModelRolePane role="formatting" draft={draft} updateDraft={updateDraft} />
            </TabsContent>

            <TabsContent value="about" className="h-screen overflow-x-hidden overflow-y-auto overscroll-contain p-5">
              <AboutPane />
            </TabsContent>
          </Tabs>
        )}
      </div>
    </main>
  );
}

type PaneProps = {
  draft: SettingsRecord;
  updateDraft: (key: string, value: SettingValue) => void;
};

function SystemPane({
  draft,
  updateDraft,
  isUpdatingAutostart,
  updateLaunchAtStart,
}: PaneProps & {
  isUpdatingAutostart: boolean;
  updateLaunchAtStart: (checked: boolean) => Promise<void>;
}) {
  const { t } = useTranslation();

  return (
    <div className="grid gap-4 h-full">
      <Card>
        <CardContent className="space-y-5">
          <SettingRow title={t("system.language.title")} description={t("system.language.description")}>
            <Select
              value={draft["system.language"] as string}
              onValueChange={(value) =>
                updateDraft("system.language", value as AppLanguage)
              }
            >
              <SelectTrigger className="w-56">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="system">{t("language.system")}</SelectItem>
                <SelectItem value="en">{t("language.en")}</SelectItem>
                <SelectItem value="es">{t("language.es")}</SelectItem>
              </SelectContent>
            </Select>
          </SettingRow>
          <SettingRow title={t("system.launchAtStart.title")} description={t("system.launchAtStart.description")}>
            <Switch
              checked={draft["system.launch_at_start"] as boolean}
              disabled={isUpdatingAutostart}
              onCheckedChange={(checked) => void updateLaunchAtStart(checked)}
            />
          </SettingRow>
          <SettingRow title={t("system.saveTextHistory.title")} description={t("system.saveTextHistory.description")}>
            <Switch checked={false} disabled />
          </SettingRow>
          <SettingRow title={t("system.saveInputAudio.title")} description={t("system.saveInputAudio.description")}>
            <Switch
              checked={draft["system.save_input_audio"] as boolean}
              onCheckedChange={(checked) => updateDraft("system.save_input_audio", checked)}
            />
          </SettingRow>
        </CardContent>
      </Card>

      <Card className="h-full">
        <CardHeader>
          <CardTitle>{t("system.stats.title")}</CardTitle>
        </CardHeader>
        <CardContent className="grid grid-cols-3 items-start gap-3">
          <StatRow label={t("system.stats.transcripts")} value="0" />
          <StatRow label={t("system.stats.words")} value="0" />
          <StatRow label={t("system.stats.minutes")} value="0" />
        </CardContent>
      </Card>
    </div>
  );
}

function RecordingPane({
  draft,
  updateDraft,
  disabled,
  inputDevices,
  soundOptions,
  previewingSoundKey,
  previewSound,
  permissions,
  isRefreshingPermissions,
  permissionError,
  refreshPermissions,
}: PaneProps & {
  disabled: boolean;
  inputDevices: string[];
  soundOptions: SoundOption[];
  previewingSoundKey: string | null;
  previewSound: (key: string, path: string | null) => Promise<void>;
  permissions: PermissionsStatus | null;
  isRefreshingPermissions: boolean;
  permissionError: string | null;
  refreshPermissions: () => Promise<void>;
}) {
  const { t } = useTranslation();
  const selectedInputDevice = draft["recording.microphone.input_device"];
  const selectedInputDeviceValue =
    typeof selectedInputDevice === "string" ? selectedInputDevice : SYSTEM_DEFAULT_INPUT_DEVICE;
  const selectedDeviceUnavailable =
    typeof selectedInputDevice === "string" && !inputDevices.includes(selectedInputDevice);

  return (
    <div className="grid gap-4">
      <Card>
        <CardHeader>
          <CardTitle>{t("permissions.title")}</CardTitle>
          <CardDescription>{t("permissions.description")}</CardDescription>
        </CardHeader>
        <CardContent className="flex flex-wrap items-center gap-3">
          <PermissionBadge label={t("permissions.microphone")} state={permissions?.microphone} />
          <PermissionBadge label={t("permissions.accessibility")} state={permissions?.accessibility} />
          <Button
            variant="outline"
            size="sm"
            disabled={isRefreshingPermissions}
            onClick={() => void refreshPermissions()}
          >
            {isRefreshingPermissions ? t("common.refreshing") : t("common.refresh")}
          </Button>
          {permissionError ? (
            <span className="text-xs text-destructive">{permissionError}</span>
          ) : null}
        </CardContent>
      </Card>

      <Card>
        <CardContent className="space-y-5">
          <SettingRow title={t("recording.mode.title")} description={t("recording.mode.description")}>
            <Select
              value={draft["recording.mode"] as string}
              disabled={disabled}
              onValueChange={(value) =>
                updateDraft("recording.mode", value as InteractionMode)
              }
            >
              <SelectTrigger className="w-52">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="push_to_talk">{t("recording.mode.pushToTalk")}</SelectItem>
                <SelectItem value="toggle">{t("recording.mode.toggle")}</SelectItem>
              </SelectContent>
            </Select>
          </SettingRow>

          <SettingRow title={t("recording.hotkey.title")} description={t("recording.hotkey.description")}>
            <HotkeyCapture
              shortcut={draft["recording.hotkey"] as string}
              disabled={disabled}
              onChange={(shortcut) =>
                updateDraft("recording.hotkey", shortcut)
              }
            />
          </SettingRow>

          <SettingRow title={t("recording.microphone.title")} description={t("recording.microphone.description")}>
            <Select
              disabled={disabled}
              value={selectedInputDeviceValue}
              onValueChange={(value) =>
                updateDraft(
                  "recording.microphone.input_device",
                  value === SYSTEM_DEFAULT_INPUT_DEVICE ? null : value
                )
              }
            >
              <SelectTrigger className="w-64">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value={SYSTEM_DEFAULT_INPUT_DEVICE}>{t("common.systemDefault")}</SelectItem>
                {inputDevices.map((device) => (
                  <SelectItem key={device} value={device}>
                    {device}
                  </SelectItem>
                ))}
                {selectedDeviceUnavailable ? (
                  <SelectItem value={selectedInputDevice as string} disabled>
                    {selectedInputDevice} ({t("common.unavailable")})
                  </SelectItem>
                ) : null}
              </SelectContent>
            </Select>
          </SettingRow>

          <Separator />

          <SettingRow title={t("recording.soundStart.title")} description={t("recording.soundStart.description")}>
            <SoundControl
              settingKey="recording.sounds.start"
              slot="start"
              value={draft["recording.sounds.start"] as string | null}
              options={soundOptions}
              disabled={disabled}
              previewingSoundKey={previewingSoundKey}
              onChange={(value) => updateDraft("recording.sounds.start", value)}
              onPreview={previewSound}
            />
          </SettingRow>

          <SettingRow title={t("recording.soundEnd.title")} description={t("recording.soundEnd.description")}>
            <SoundControl
              settingKey="recording.sounds.stop"
              slot="stop"
              value={draft["recording.sounds.stop"] as string | null}
              options={soundOptions}
              disabled={disabled}
              previewingSoundKey={previewingSoundKey}
              onChange={(value) => updateDraft("recording.sounds.stop", value)}
              onPreview={previewSound}
            />
          </SettingRow>

          <SettingRow title={t("recording.soundError.title")} description={t("recording.soundError.description")}>
            <SoundControl
              settingKey="recording.sounds.error"
              slot="error"
              value={draft["recording.sounds.error"] as string | null}
              options={soundOptions}
              disabled={disabled}
              previewingSoundKey={previewingSoundKey}
              onChange={(value) => updateDraft("recording.sounds.error", value)}
              onPreview={previewSound}
            />
          </SettingRow>

          <SettingRow title={t("recording.pauseMedia.title")} description={t("recording.pauseMedia.description")}>
            <Switch
              checked={Boolean(draft["recording.pause_media"])}
              disabled={disabled}
              onCheckedChange={(checked) => updateDraft("recording.pause_media", checked)}
            />
          </SettingRow>
        </CardContent>
      </Card>
    </div>
  );
}

function PermissionBadge({ label, state }: { label: string; state?: PermissionState }) {
  const isGranted = state === "granted";
  const Icon = isGranted ? ShieldCheck : ShieldX;
  const tone = state === undefined ? "default" : isGranted ? "success" : "danger";

  return (
    <SettingsBadge tone={tone} icon={<Icon className="size-3" aria-hidden="true" />}>
      {label}
    </SettingsBadge>
  );
}

function SoundControl({
  settingKey,
  slot,
  value,
  options,
  disabled,
  previewingSoundKey,
  onChange,
  onPreview,
}: {
  settingKey: string;
  slot: SoundSlot;
  value: string | null;
  options: SoundOption[];
  disabled: boolean;
  previewingSoundKey: string | null;
  onChange: (value: string | null) => void;
  onPreview: (key: string, path: string | null) => Promise<void>;
}) {
  const { t } = useTranslation();
  const isPreviewing = previewingSoundKey === settingKey;

  return (
    <div className="flex items-center gap-2">
      <SoundSelect
        value={value}
        slot={slot}
        options={options}
        disabled={disabled}
        onChange={onChange}
      />
      <Button
        variant="outline"
        size="icon"
        disabled={disabled || !isTauri() || value === null || isPreviewing}
        onClick={() => void onPreview(settingKey, value)}
        aria-label={t("common.preview")}
        title={t("common.preview")}
      >
        <Play className="size-4" aria-hidden="true" />
      </Button>
    </div>
  );
}

function SoundSelect({
  value,
  slot,
  options,
  disabled,
  onChange,
}: {
  value: string | null;
  slot: SoundSlot;
  options: SoundOption[];
  disabled: boolean;
  onChange: (value: string | null) => void;
}) {
  const { t } = useTranslation();
  const noneValue = "__none__";
  const customValue = "__custom__";
  const optionPaths = options.map((option) => soundOptionPath(option));
  const isCustomSelected = value !== null && !optionPaths.includes(value);
  const selectValue = value === null ? noneValue : value;

  const handleChange = async (next: string) => {
    if (next === noneValue) {
      onChange(null);
      return;
    }

    if (next !== customValue) {
      onChange(next);
      return;
    }

    if (!isTauri()) {
      return;
    }

    try {
      const selected = await open({
        multiple: false,
        directory: false,
        filters: [{ name: "WAV", extensions: ["wav"] }],
      });

      if (typeof selected !== "string") {
        return;
      }

      const imported = await tauriInvoke<string>("import_custom_sound", { path: selected, slot });
      onChange(imported);
    } catch (error) {
      logger.error("failed to import custom sound", {
        error: error instanceof Error ? error.message : String(error),
      });
    }
  };

  return (
    <Select
      value={selectValue}
      disabled={disabled}
      onValueChange={(next) => void handleChange(next)}
    >
      <SelectTrigger className="w-64">
        <SelectValue />
      </SelectTrigger>
      <SelectContent>
        <SelectItem value={noneValue}>{t("common.none")}</SelectItem>
        {options.map((option) => (
          <SelectItem key={option.id} value={soundOptionPath(option)}>
            {option.label}
          </SelectItem>
        ))}
        {isCustomSelected ? (
          <SelectItem value={value}>
            <FileNameLabel fileName={fileNameFromPath(value)} />
          </SelectItem>
        ) : null}
        <SelectItem value={customValue}>{t("common.custom")}</SelectItem>
      </SelectContent>
    </Select>
  );
}

function soundOptionPath(option: SoundOption): string {
  return `sounds/${option.file}`;
}

function fileNameFromPath(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).pop() ?? path;
}

function FileNameLabel({ fileName }: { fileName: string }) {
  const extensionIndex = fileName.lastIndexOf(".");
  const extension = extensionIndex > 0 ? fileName.slice(extensionIndex) : "";
  const base = extensionIndex > 0 ? fileName.slice(0, extensionIndex) : fileName;

  return (
    <span className="flex min-w-0 max-w-full items-center">
      <span className="min-w-0 truncate">{base}</span>
      {extension ? <span className="shrink-0">{extension}</span> : null}
    </span>
  );
}

type ModelRole = "stt" | "formatting";

type ProviderSettingsView = {
  id: string;
  name: string;
  kind: string;
  config: Record<string, unknown>;
  override_config: Record<string, unknown>;
  effective_config: Record<string, unknown>;
  config_schema: Record<string, unknown>;
};

type CatalogModelView = {
  id: string;
  provider_id: string;
  role: ModelRole;
  external_model_id: string;
  display_name: string;
  config: Record<string, unknown>;
  config_schema: Record<string, unknown>;
};

type UserModelView = {
  id: string;
  role: ModelRole;
  provider_id: string;
  provider_name: string;
  provider_kind: string;
  provider_config: Record<string, unknown>;
  provider_override_config: Record<string, unknown>;
  provider_effective_config: Record<string, unknown>;
  provider_config_schema: Record<string, unknown>;
  model_id: string;
  external_model_id: string;
  model_display_name: string;
  display_name: string;
  config: Record<string, unknown>;
  override_config: Record<string, unknown>;
  effective_config: Record<string, unknown>;
  config_schema: Record<string, unknown>;
  is_active: boolean;
};

type ModelHealthStatus = "healthy" | "unhealthy" | "unknown";
type ModelHealthView = {
  id: string;
  role: ModelRole;
  display_name: string;
  provider_name: string;
  is_active: boolean;
  health: ModelHealthStatus;
};

type RoleModelSettings = {
  role: ModelRole;
  model_id: string | null;
  providers: ProviderSettingsView[];
  models: CatalogModelView[];
  user_models: UserModelView[];
};

type ProviderModelOption = {
  id: string;
  name: string;
};

function ModelRolePane({
  role,
  draft,
  updateDraft,
}: {
  role: ModelRole;
  draft: SettingsRecord;
  updateDraft: (key: string, value: SettingValue) => void;
}) {
  const { t } = useTranslation();
  const transformEnabled = draft["models.formatting.enabled"] !== false;
  const isFormatting = role === "formatting";

  return (
    <div className="grid gap-4">
      <ModelRoleCard
        role={role}
        title={t("models.cardTitle")}
        description={isFormatting ? t("models.formattingDescription") : t("models.sttDescription")}
        transformEnabled={isFormatting ? transformEnabled : undefined}
        onTransformEnabledChange={isFormatting ? (checked) => updateDraft("models.formatting.enabled", checked) : undefined}
      />
    </div>
  );
}

function ModelRoleCard({
  role,
  title,
  description,
  transformEnabled,
  onTransformEnabledChange,
}: {
  role: ModelRole;
  title: string;
  description: string;
  transformEnabled?: boolean;
  onTransformEnabledChange?: (checked: boolean) => void;
}) {
  const { t } = useTranslation();
  const [settings, setSettings] = useState<RoleModelSettings | null>(null);
  const [modelHealth, setModelHealth] = useState<Record<string, ModelHealthStatus>>({});
  const [isAdding, setIsAdding] = useState(false);
  const [editingModelId, setEditingModelId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const isRoleDisabled = role === "formatting" && transformEnabled === false;

  const load = useCallback(async () => {
    if (!isTauri()) {
      return;
    }
    setError(null);
    try {
      setSettings(await tauriInvoke<RoleModelSettings>("list_model_settings", { role }));
    } catch (error) {
      setError(error instanceof Error ? error.message : String(error));
    }
  }, [role]);

  useEffect(() => {
    void load();
  }, [load]);

  useTauriEvent<SettingsChangedEvent>("settings:changed", (event) => {
    if (event.keys.includes("models.active") || event.keys.includes("models.health")) {
      void load();
    }
  });

  const providers = settings?.providers ?? [];
  const catalogModels = settings?.models ?? [];
  const userModels = settings?.user_models ?? [];

  useEffect(() => {
    if (!isTauri() || userModels.length === 0) {
      setModelHealth({});
      return;
    }
    let cancelled = false;
    void tauriInvoke<ModelHealthView[]>("list_model_health", { role }).then(
      (models) => {
        if (!cancelled) {
          setModelHealth(Object.fromEntries(models.map((model) => [model.id, model.health] as const)));
        }
      },
      () => {
        if (!cancelled) {
          setModelHealth(Object.fromEntries(userModels.map((model) => [model.id, "unknown"] as const)));
        }
      }
    );
    return () => {
      cancelled = true;
    };
  }, [role, userModels]);

  const handleSaved = (modelId: string) => {
    setIsAdding(false);
    setEditingModelId(null);
    void load();
  };

  const handleSelected = () => {
    void load();
  };

  const reloadSettings = useCallback(async () => {
    if (!isTauri()) {
      return undefined;
    }
    setError(null);
    try {
      const next = await tauriInvoke<RoleModelSettings>("list_model_settings", { role });
      setSettings(next);
      return next;
    } catch (error) {
      setError(error instanceof Error ? error.message : String(error));
      return undefined;
    }
  }, [role]);

  return (
    <Card>
      <CardHeader className="flex flex-row items-start justify-between gap-4">
        <div className="min-w-0 select-none space-y-1.5">
          <CardTitle>{title}</CardTitle>
          <CardDescription>{description}</CardDescription>
        </div>
        {role === "formatting" && onTransformEnabledChange ? (
          <div className="flex shrink-0 items-center gap-2 pt-0.5">
            <Label htmlFor="models-formatting-enabled" className="text-xs text-muted-foreground">
              {t("models.enabled")}
            </Label>
            <Switch
              id="models-formatting-enabled"
              checked={transformEnabled ?? true}
              onCheckedChange={onTransformEnabledChange}
            />
          </div>
        ) : null}
      </CardHeader>
      <CardContent className={`space-y-3 ${isRoleDisabled ? "opacity-50" : ""}`}>
        {userModels.length === 0 && !isAdding ? (
          <div className="rounded-lg bg-muted/40 p-3 text-sm text-muted-foreground">
            {t("models.noModel")}
          </div>
        ) : null}
        {userModels.map((model) => (
          <ModelItem
            key={model.id}
            role={role}
            model={model}
            catalogModels={catalogModels}
            providers={providers}
            isSelected={model.is_active}
            health={modelHealth[model.id] ?? "unknown"}
            isLocked={isRoleDisabled || (editingModelId !== null && editingModelId !== model.id)}
            onSaved={handleSaved}
            onSelected={handleSelected}
            onEditingChange={(editing) => setEditingModelId(editing ? model.id : null)}
            reloadSettings={reloadSettings}
            onDeleted={() => {
              void load();
            }}
          />
        ))}
        {isAdding ? (
          <ModelItem
            role={role}
            model={null}
            catalogModels={catalogModels}
            providers={providers}
            isSelected={false}
            health="unknown"
            isLocked={isRoleDisabled || (editingModelId !== null && editingModelId !== "__new__")}
            onSaved={handleSaved}
            onSelected={handleSelected}
            onEditingChange={(editing) => setEditingModelId(editing ? "__new__" : null)}
            reloadSettings={reloadSettings}
            onCancel={() => {
              setEditingModelId(null);
              setIsAdding(false);
            }}
          />
        ) : (
          <div className="flex justify-end">
            <Button
              variant="outline"
              size="sm"
              disabled={isRoleDisabled || editingModelId !== null}
              onClick={() => {
                setEditingModelId("__new__");
                setIsAdding(true);
              }}
            >
              <Plus className="size-4" aria-hidden="true" />
              {t("models.addModel")}
            </Button>
          </div>
        )}
        {error ? <p className="text-xs text-destructive">{error}</p> : null}
      </CardContent>
    </Card>
  );
}

function ModelItem({
  role,
  model,
  catalogModels,
  providers,
  isSelected,
  health,
  isLocked,
  onSaved,
  onSelected,
  onEditingChange,
  reloadSettings,
  onDeleted,
  onCancel,
}: {
  role: ModelRole;
  model: UserModelView | null;
  catalogModels: CatalogModelView[];
  providers: ProviderSettingsView[];
  isSelected: boolean;
  health: ModelHealthStatus;
  isLocked: boolean;
  onSaved: (modelId: string) => void;
  onSelected: () => void;
  onEditingChange: (editing: boolean) => void;
  reloadSettings: () => Promise<RoleModelSettings | undefined>;
  onDeleted?: () => void;
  onCancel?: () => void;
}) {
  const { t } = useTranslation();
  const [isEditing, setIsEditing] = useState(model === null);
  const initialProviderId = model?.provider_id ?? providers[0]?.id ?? "";
  const [providerId, setProviderId] = useState(initialProviderId);
  const selectedProvider = providers.find((provider) => provider.id === providerId) ?? providers[0];
  const initialCatalogModel = model
    ? catalogModels.find((item) => item.id === model.model_id)
    : catalogModels.find((item) => item.provider_id === initialProviderId);
  const initialProviderConfig = model
    ? nonEmptyObject(model.provider_override_config) ? model.provider_override_config : model.provider_config
    : selectedProvider?.config ?? {};
  const initialModelConfig = model
    ? nonEmptyObject(model.override_config) ? model.override_config : model.config
    : initialCatalogModel?.config ?? {};
  const [providerConfigText, setProviderConfigText] = useState(formatJson(initialProviderConfig));
  const [modelConfigText, setModelConfigText] = useState(formatJson(initialModelConfig));
  const [selectedCatalogModelId, setSelectedCatalogModelId] = useState(model?.model_id ?? initialCatalogModel?.id ?? "");
  const [displayName, setDisplayName] = useState(
    model?.display_name ?? initialCatalogModel?.display_name ?? initialCatalogModel?.external_model_id ?? ""
  );
  const modelOptionsForProvider = useCallback(
    (nextProviderId: string, sourceModels: CatalogModelView[] = catalogModels): ProviderModelOption[] =>
      sourceModels
        .filter((item) => item.provider_id === nextProviderId)
        .map((item) => ({
          id: item.id,
          name: item.display_name || item.external_model_id,
        })),
    [catalogModels]
  );
  const catalogModelById = useCallback(
    (catalogModelId: string, sourceModels: CatalogModelView[] = catalogModels) =>
      sourceModels.find((item) => item.id === catalogModelId),
    [catalogModels]
  );
  const [options, setOptions] = useState<ProviderModelOption[]>(() => modelOptionsForProvider(initialProviderId));
  const [isRefreshing, setIsRefreshing] = useState(false);
  const [isSaving, setIsSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const autoRefreshAttempted = useRef<Set<string>>(new Set());

  const providerConfig = parseJsonObject(providerConfigText);
  const modelConfig = parseJsonObject(modelConfigText);
  const authType = getNestedString(providerConfig.value, ["auth", "type"]);
  const apiKey = getNestedString(providerConfig.value, ["auth", "api_key"]);
  const requiresApiKey = authType === "bearer_api_key";
  const canRefreshModels = Boolean(providerId && providerConfig.value && (!requiresApiKey || apiKey.trim().length > 0));

  const updateProviderConfig = (next: Record<string, unknown>) => {
    setProviderConfigText(formatJson(next));
  };

  const updateProviderField = (path: string[], value: string) => {
    const source = providerConfig.value ?? {};
    updateProviderConfig(setNestedValue(source, path, value));
  };

  const changeProvider = (nextProviderId: string) => {
    setProviderId(nextProviderId);
    const provider = providers.find((item) => item.id === nextProviderId);
    const nextOptions = modelOptionsForProvider(nextProviderId);
    const firstOption = nextOptions[0];
    const firstCatalogModel = firstOption ? catalogModelById(firstOption.id) : undefined;
    setProviderConfigText(formatJson(provider?.config ?? {}));
    setSelectedCatalogModelId(firstOption?.id ?? "");
    setDisplayName(firstOption?.name ?? "");
    setModelConfigText(formatJson(firstCatalogModel?.config ?? {}));
    setOptions(nextOptions);
  };

  const refreshModels = useCallback(async () => {
    if (!providerId || !providerConfig.value || !canRefreshModels) {
      setError(t("models.invalidJson"));
      return;
    }
    setIsRefreshing(true);
    setError(null);
    try {
      const refreshed = await tauriInvoke<ProviderModelOption[]>("refresh_provider_models", {
        role,
        providerId,
        providerConfigOverride: providerConfig.value,
      });
      const reloaded = await reloadSettings();
      const reloadedCatalogModels = reloaded?.models ?? catalogModels;
      const nextOptions = mergeModelOptions(modelOptionsForProvider(providerId, reloadedCatalogModels), refreshed);
      const nextSelectedModelId =
        selectedCatalogModelId && nextOptions.some((option) => option.id === selectedCatalogModelId)
          ? selectedCatalogModelId
          : nextOptions[0]?.id ?? "";
      const nextSelectedModel = nextSelectedModelId ? catalogModelById(nextSelectedModelId, reloadedCatalogModels) : undefined;
      setOptions(nextOptions);
      setSelectedCatalogModelId(nextSelectedModelId);
      setDisplayName(nextSelectedModel?.display_name || nextSelectedModel?.external_model_id || "");
      setModelConfigText(formatJson(nextSelectedModel?.config ?? {}));
    } catch (error) {
      setError(`${t("models.refreshError")}: ${error instanceof Error ? error.message : String(error)}`);
    } finally {
      setIsRefreshing(false);
    }
  }, [
    providerId,
    providerConfig.value,
    canRefreshModels,
    role,
    t,
    modelOptionsForProvider,
    reloadSettings,
    catalogModels,
    selectedCatalogModelId,
    catalogModelById,
  ]);

  useEffect(() => {
    if (!isEditing || !providerId || !canRefreshModels || options.length > 0 || autoRefreshAttempted.current.has(providerId)) {
      return;
    }
    autoRefreshAttempted.current.add(providerId);
    void refreshModels();
  }, [isEditing, providerId, canRefreshModels, options.length, refreshModels]);

  useEffect(() => {
    if (selectedCatalogModelId || options.length === 0) {
      return;
    }
    const first = options[0];
    setSelectedCatalogModelId(first.id);
    setDisplayName(first.name);
    setModelConfigText(formatJson(catalogModelById(first.id)?.config ?? {}));
  }, [selectedCatalogModelId, options, catalogModelById]);

  const save = async () => {
    if (!providerConfig.value || !modelConfig.value) {
      setError(t("models.invalidJson"));
      return;
    }
    setIsSaving(true);
    setError(null);
    try {
      const modelId = await tauriInvoke<string>("save_model_item", {
        role,
        providerId,
        modelId: selectedCatalogModelId,
        displayName: displayName || null,
        providerConfigOverride: providerConfig.value,
        modelConfigOverride: modelConfig.value,
      });
      setIsEditing(false);
      onSaved(modelId);
    } catch (error) {
      setError(`${t("models.saveError")}: ${error instanceof Error ? error.message : String(error)}`);
    } finally {
      setIsSaving(false);
    }
  };

  const remove = async () => {
    if (!model || isLocked) {
      return;
    }
    setIsSaving(true);
    setError(null);
    try {
      await tauriInvoke<void>("delete_model_item", { role, modelId: model.id });
      onDeleted?.();
    } catch (error) {
      setError(`${t("models.saveError")}: ${error instanceof Error ? error.message : String(error)}`);
    } finally {
      setIsSaving(false);
    }
  };

  const select = async () => {
    if (!model || isSelected || isLocked) {
      return;
    }
    setIsSaving(true);
    setError(null);
    try {
      await tauriInvoke<void>("select_model", { role, modelId: model.id });
      onSelected();
    } catch (error) {
      setError(`${t("models.saveError")}: ${error instanceof Error ? error.message : String(error)}`);
    } finally {
      setIsSaving(false);
    }
  };

  if (!isEditing && model) {
    return (
      <div
        className={`group flex cursor-pointer items-center justify-between gap-3 rounded-lg border p-3 transition-colors ${
          isLocked
            ? "cursor-default border-transparent bg-muted/40 opacity-60 hover:border-transparent"
            : isSelected
            ? "border-border bg-muted/40 hover:border-border/50"
            : "border-transparent bg-muted/60 hover:border-border/50"
        }`}
        aria-disabled={isLocked || undefined}
        role={isLocked ? undefined : "button"}
        tabIndex={isLocked ? -1 : 0}
        onClick={isLocked ? undefined : () => void select()}
        onKeyDown={
          isLocked
            ? undefined
            : (event) => {
                if (event.key === "Enter" || event.key === " ") {
                  event.preventDefault();
                  void select();
                }
              }
        }
      >
        <div className="min-w-0 select-none">
          <div className="flex min-w-0 items-center gap-2">
            <div className="truncate text-sm font-medium">{model.provider_name}</div>
          </div>
          <div className="truncate text-xs text-muted-foreground">
            {model.display_name || model.model_display_name || model.external_model_id}
          </div>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          {!isLocked ? (
            <div className="flex items-center gap-2 opacity-0 transition-opacity group-hover:opacity-100 group-focus-within:opacity-100">
              <Button
                variant="ghost"
                size="icon"
                aria-label={t("common.edit")}
                title={t("common.edit")}
                onClick={(event) => {
                  event.stopPropagation();
                  setIsEditing(true);
                  onEditingChange(true);
                }}
              >
                <SquarePen className="size-4" aria-hidden="true" />
              </Button>
              <Button
                variant="ghost"
                size="icon"
                disabled={isSaving}
                aria-label={t("common.remove")}
                title={t("common.remove")}
                onClick={(event) => {
                  event.stopPropagation();
                  void remove();
                }}
              >
                <Trash2 className="size-4" aria-hidden="true" />
              </Button>
            </div>
          ) : null}
          <ModelHealthIcon health={health} isSelected={isSelected} />
        </div>
      </div>
    );
  }

  return (
    <div className="space-y-3 rounded-lg bg-muted/35 p-3">
      <div className="grid gap-3 sm:grid-cols-[minmax(10rem,max-content)_minmax(14rem,1fr)] sm:items-end">
        <div className="min-w-0 max-w-full space-y-1">
          <Label>{t("models.provider")}</Label>
          <Select value={providerId} onValueChange={changeProvider}>
            <SelectTrigger className="w-full">
              <SelectValue placeholder={t("models.noProvider")} />
            </SelectTrigger>
            <SelectContent>
              {providers.map((provider) => (
                <SelectItem key={provider.id} value={provider.id}>
                  {provider.name}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
        <div className="grid min-w-0 grid-cols-[minmax(14rem,1fr)_auto] items-end gap-2">
          <div className="min-w-0 space-y-1">
            <Label>{t("models.model")}</Label>
            <Select
              value={selectedCatalogModelId}
              disabled={options.length === 0}
              onValueChange={(value) => {
                const selectedModel = catalogModelById(value);
                setSelectedCatalogModelId(value);
                setDisplayName(selectedModel?.display_name || selectedModel?.external_model_id || value);
                setModelConfigText(formatJson(selectedModel?.config ?? {}));
              }}
            >
              <SelectTrigger className="w-full">
                <SelectValue placeholder={t("models.refreshModels")} />
              </SelectTrigger>
              <SelectContent>
                {options.map((option) => (
                  <SelectItem key={option.id} value={option.id}>
                    {option.name}
                  </SelectItem>
                ))}
                {selectedCatalogModelId && !options.some((option) => option.id === selectedCatalogModelId) ? (
                  <SelectItem value={selectedCatalogModelId}>{selectedCatalogModelId}</SelectItem>
                ) : null}
              </SelectContent>
            </Select>
          </div>
          <Button
            variant="outline"
            size="icon"
            disabled={!canRefreshModels || isRefreshing}
            onClick={() => void refreshModels()}
            aria-label={t("models.refreshModels")}
            title={t("models.refreshModels")}
          >
            <RefreshCw className={`size-4 ${isRefreshing ? "animate-spin" : ""}`} aria-hidden="true" />
          </Button>
        </div>
      </div>

      {requiresApiKey ? (
        <div className="space-y-1">
          <Label>{t("models.apiKey")}</Label>
          <Input
            type="password"
            value={apiKey}
            onChange={(event) => updateProviderField(["auth", "api_key"], event.target.value)}
          />
        </div>
      ) : null}

      <div className="grid gap-3 sm:grid-cols-2">
        <div className="space-y-1">
          <Label>{t("models.providerOverride")}</Label>
          <JsonTextarea
            value={providerConfigText}
            onChange={setProviderConfigText}
          />
        </div>
        <div className="space-y-1">
          <Label>{t("models.modelOverride")}</Label>
          <JsonTextarea
            value={modelConfigText}
            onChange={setModelConfigText}
          />
        </div>
      </div>

      <div className="flex flex-wrap items-center justify-between gap-3">
        <div />
        <div className="flex gap-2">
          <Button
            variant="ghost"
            size="sm"
            onClick={() => {
              if (model) {
                setIsEditing(false);
                onEditingChange(false);
              } else {
                onCancel?.();
              }
            }}
          >
            {t("common.cancel")}
          </Button>
          <Button size="sm" disabled={isSaving || !providerId || !selectedCatalogModelId.trim()} onClick={() => void save()}>
            {t("common.save")}
          </Button>
        </div>
      </div>
      {error ? <p className="text-xs text-destructive">{error}</p> : null}
    </div>
  );
}

function ModelHealthIcon({ health, isSelected }: { health: ModelHealthStatus; isSelected: boolean }) {
  if (health === "unhealthy") {
    return <TriangleAlert className="size-4 text-destructive" aria-hidden="true" />;
  }
  if (health === "healthy") {
    return <Check className={`size-4 ${isSelected ? "text-success" : "text-muted-foreground"}`} aria-hidden="true" />;
  }
  return null;
}

function AboutPane() {
  const { t } = useTranslation();

  return (
    <Card>
      <CardHeader>
        <CardTitle>{t("about.title")}</CardTitle>
        <CardDescription>{t("about.description")}</CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        <SettingRow title={t("about.version.title")} description={t("about.version.description")}>
          <Badge variant="outline">{APP_VERSION}</Badge>
        </SettingRow>
        <SettingRow title={t("about.website.title")} description={t("about.website.description")}>
          <Button variant="link" size="sm" disabled>
            {t("common.website")}
          </Button>
        </SettingRow>
        <SettingRow title={t("about.github.title")} description={t("about.github.description")}>
          <Button variant="link" size="sm" disabled>
            GitHub
          </Button>
        </SettingRow>
      </CardContent>
    </Card>
  );
}

function SettingRow({
  title,
  description,
  children,
}: {
  title: string;
  description?: string;
  children: React.ReactNode;
}) {
  return (
    <div className="grid p-0 sm:grid-cols-[1fr_auto] sm:items-center">
      <div className="space-y-1">
        <Label className="text-sm font-medium">{title}</Label>
        {description ? <p className="text-xs text-muted-foreground">{description}</p> : null}
      </div>
      <div className="flex justify-start sm:justify-end">{children}</div>
    </div>
  );
}

function StatRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex flex-col items-center justify-center gap-1 rounded-lg bg-muted/45 px-3 py-2 text-center">
      <span className="text-xs text-muted-foreground">{label}</span>
      <span className="text-xl font-semibold tabular-nums">{value}</span>
    </div>
  );
}

function formatJson(value: unknown): string {
  return JSON.stringify(value ?? {}, null, 2);
}

function mergeModelOptions(base: ProviderModelOption[], next: ProviderModelOption[]): ProviderModelOption[] {
  const byId = new Map<string, ProviderModelOption>();
  for (const option of base) {
    byId.set(option.id, option);
  }
  for (const option of next) {
    byId.set(option.id, option);
  }
  return Array.from(byId.values());
}

function parseJsonObject(raw: string): { value: Record<string, unknown> | null } {
  try {
    const parsed = JSON.parse(raw);
    if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) {
      return { value: parsed as Record<string, unknown> };
    }
    return { value: null };
  } catch {
    return { value: null };
  }
}

function nonEmptyObject(value: Record<string, unknown>): boolean {
  return Object.keys(value).length > 0;
}

function getNestedString(source: Record<string, unknown> | null, path: string[]): string {
  let current: unknown = source;
  for (const key of path) {
    if (!current || typeof current !== "object" || Array.isArray(current)) {
      return "";
    }
    current = (current as Record<string, unknown>)[key];
  }
  return typeof current === "string" ? current : "";
}

function setNestedValue(source: Record<string, unknown>, path: string[], value: string): Record<string, unknown> {
  const clone = structuredClone(source);
  let current: Record<string, unknown> = clone;
  path.forEach((key, index) => {
    if (index === path.length - 1) {
      current[key] = value;
      return;
    }
    const next = current[key];
    if (!next || typeof next !== "object" || Array.isArray(next)) {
      current[key] = {};
    }
    current = current[key] as Record<string, unknown>;
  });
  return clone;
}
