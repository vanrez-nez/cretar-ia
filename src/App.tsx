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
import { SettingsBadge } from "@/components/settings-badge";
import { tauriInvoke, useTauriEvent } from "@/hooks/useTauriIPC";
import i18n, { resolveAppLocale } from "@/i18n";
import { logger } from "@/lib/logger";
import { setLaunchAtStart } from "@/settings/autostart";
import type { SettingsRecord, SettingValue } from "@/settings/schema";
import { useSettingsStore } from "./stores/settingsStore";
import type { AppLanguage, InteractionMode, PermissionState, PermissionsStatus } from "./lib/types";
import { Play, ShieldCheck, ShieldX } from "lucide-react";

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
    if (!event.keys.includes("recording.microphone.input_device")) {
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
    <main className="h-screen overflow-hidden bg-background text-foreground">
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
          <Tabs defaultValue="system" orientation="vertical" className="h-full w-full gap-0 overflow-x-hidden">
            <SettingsNavigation />

            <TabsContent value="system" className="ml-44 h-screen overflow-x-hidden overflow-y-auto overscroll-contain p-5 pl-0">
              <SystemPane
                draft={draft}
                updateDraft={updateDraft}
                isUpdatingAutostart={isUpdatingAutostart}
                updateLaunchAtStart={updateLaunchAtStart}
              />
            </TabsContent>

            <TabsContent value="recording" className="ml-44 h-screen overflow-x-hidden overflow-y-auto overscroll-contain p-5 pl-0">
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

            <TabsContent value="models" className="ml-44 h-screen overflow-x-hidden overflow-y-auto overscroll-contain p-5 pl-0">
              <ModelsPane draft={draft} />
            </TabsContent>

            <TabsContent value="about" className="ml-44 h-screen overflow-x-hidden overflow-y-auto overscroll-contain p-5 pl-0">
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
            <Switch checked={false} disabled />
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

function ModelsPane({ draft }: { draft: SettingsRecord }) {
  const { t } = useTranslation();
  const provider = draft["models.stt.provider"] as string;
  const providerName = useMemo(() => providerLabel(provider, t), [provider, t]);

  return (
    <div className="grid gap-4">
      <Card>
        <CardHeader>
          <CardTitle>{t("models.sttTitle")}</CardTitle>
          <CardDescription>{t("models.sttDescription")}</CardDescription>
        </CardHeader>
        <CardContent className="space-y-3">
          <ProviderItem name={providerName} detail={(draft["models.stt.openrouter.model"] as string) || t("models.noModel")} />
          <Button variant="outline" size="sm" disabled>
            {t("models.addProvider")}
          </Button>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>{t("models.formattingTitle")}</CardTitle>
          <CardDescription>{t("models.formattingDescription")}</CardDescription>
        </CardHeader>
        <CardContent className="space-y-3">
          <ProviderItem name={t("models.noProvider")} detail={t("common.unavailable")} disabled />
          <Button variant="outline" size="sm" disabled>
            {t("models.addProvider")}
          </Button>
        </CardContent>
      </Card>
    </div>
  );
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

function ProviderItem({
  name,
  detail,
  disabled = false,
}: {
  name: string;
  detail: string;
  disabled?: boolean;
}) {
  const { t } = useTranslation();

  return (
    <div className="flex items-center justify-between gap-3 rounded-lg border border-border/70 bg-card/50 p-3 opacity-100 data-[disabled=true]:opacity-50" data-disabled={disabled}>
      <div className="min-w-0">
        <div className="truncate text-sm font-medium">{name}</div>
        <div className="truncate text-xs text-muted-foreground">{detail}</div>
      </div>
      <div className="flex shrink-0 gap-2">
        <Button variant="outline" size="sm" disabled>
          {t("common.edit")}
        </Button>
        <Button variant="ghost" size="sm" disabled>
          {t("common.remove")}
        </Button>
      </div>
    </div>
  );
}

function providerLabel(provider: string, t: (key: string) => string) {
  if (provider === "openrouter") {
    return t("provider.openrouter");
  }
  return provider || t("provider.fallback");
}
