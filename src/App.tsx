import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { isTauri } from "@tauri-apps/api/core";
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
import { tauriInvoke } from "@/hooks/useTauriIPC";
import i18n, { resolveAppLocale } from "@/i18n";
import { logger } from "@/lib/logger";
import { useSettingsStore } from "./stores/settingsStore";
import type { AppConfig, AppLanguage, InteractionMode, PermissionState, PermissionsStatus } from "./lib/types";
import { ShieldCheck, ShieldX } from "lucide-react";

const AUTOSAVE_DELAY_MS = 500;
const APP_VERSION = "0.1.0";
export default function App() {
  const { t } = useTranslation();
  const fetchSettings = useSettingsStore((s) => s.fetchSettings);
  const config = useSettingsStore((s) => s.config);
  const isReady = useSettingsStore((s) => s.isReady);
  const saveSettings = useSettingsStore((s) => s.updateSettings);
  const [draftConfig, setDraftConfig] = useState<AppConfig | null>(null);
  const [permissions, setPermissions] = useState<PermissionsStatus | null>(null);
  const [isRefreshingPermissions, setIsRefreshingPermissions] = useState(false);
  const [permissionError, setPermissionError] = useState<string | null>(null);
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
    if (!config) {
      return;
    }
    setDraftConfig(config);
    void i18n.changeLanguage(resolveAppLocale(config.ui?.language));
    skipNextAutosave.current = true;
    hasHydrated.current = true;
  }, [config]);

  useEffect(() => {
    if (!draftConfig || !hasHydrated.current) {
      return;
    }
    void i18n.changeLanguage(resolveAppLocale(draftConfig.ui?.language));
    if (skipNextAutosave.current) {
      skipNextAutosave.current = false;
      return;
    }

    const timeout = window.setTimeout(() => {
      void saveSettings(draftConfig).catch((error) => {
        logger.error("failed to autosave settings", {
          error: error instanceof Error ? error.message : String(error),
        });
      });
    }, AUTOSAVE_DELAY_MS);

    return () => window.clearTimeout(timeout);
  }, [draftConfig, saveSettings]);

  const updateDraft = (updater: (config: AppConfig) => AppConfig) => {
    setDraftConfig((current) => {
      if (!current) {
        return current;
      }
      return updater(current);
    });
  };

  const draft = draftConfig;

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
              <SystemPane draft={draft} updateDraft={updateDraft} />
            </TabsContent>

            <TabsContent value="recording" className="ml-44 h-screen overflow-x-hidden overflow-y-auto overscroll-contain p-5 pl-0">
              <RecordingPane
                draft={draft}
                updateDraft={updateDraft}
                disabled={!isReady}
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
  draft: AppConfig;
  updateDraft: (updater: (config: AppConfig) => AppConfig) => void;
};

function SystemPane({ draft, updateDraft }: PaneProps) {
  const { t } = useTranslation();

  return (
    <div className="grid gap-4 h-full">
      <Card>
        <CardContent className="space-y-5">
          <SettingRow title={t("system.language.title")} description={t("system.language.description")}>
            <Select
              value={draft.ui?.language ?? "system"}
              onValueChange={(value) =>
                updateDraft((config) => ({
                  ...config,
                  ui: {
                    ...config.ui,
                    language: value as AppLanguage,
                  },
                }))
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
            <Switch checked={false} disabled />
          </SettingRow>
          <SettingRow title={t("system.saveTextHistory.title")} description={t("system.saveTextHistory.description")}>
            <Switch checked={false} disabled />
          </SettingRow>
          <SettingRow title={t("system.saveInputAudio.title")} description={t("system.saveInputAudio.description")}>
            <Switch
              checked={!draft.output.cleanup_recording_after_processing}
              onCheckedChange={(checked) =>
                updateDraft((config) => ({
                  ...config,
                  output: {
                    ...config.output,
                    cleanup_recording_after_processing: !checked,
                  },
                }))
              }
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
  permissions,
  isRefreshingPermissions,
  permissionError,
  refreshPermissions,
}: PaneProps & {
  disabled: boolean;
  permissions: PermissionsStatus | null;
  isRefreshingPermissions: boolean;
  permissionError: string | null;
  refreshPermissions: () => Promise<void>;
}) {
  const { t } = useTranslation();
  const startEnabled = draft.audio_cues.start_sound !== null;
  const stopEnabled = draft.audio_cues.stop_sound !== null;
  const errorEnabled = draft.audio_cues.error_sound !== null;

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
              value={draft.interaction.mode}
              disabled={disabled}
              onValueChange={(value) =>
                updateDraft((config) => ({
                  ...config,
                  interaction: {
                    ...config.interaction,
                    mode: value as InteractionMode,
                  },
                }))
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
              shortcut={draft.interaction.shortcut}
              disabled={disabled}
              onChange={(shortcut) =>
                updateDraft((config) => ({
                  ...config,
                  interaction: {
                    ...config.interaction,
                    shortcut,
                  },
                }))
              }
            />
          </SettingRow>

          <SettingRow title={t("recording.microphone.title")} description={t("recording.microphone.description")}>
            <Select disabled value="default">
              <SelectTrigger className="w-64">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="default">{t("common.default")}</SelectItem>
              </SelectContent>
            </Select>
          </SettingRow>

          <Separator />

          <SettingRow title={t("recording.soundStart.title")} description={t("recording.soundStart.description")}>
            <div className="flex items-center gap-2">
              <Switch
                checked={startEnabled}
                onCheckedChange={(checked) =>
                  updateDraft((config) => ({
                    ...config,
                    audio_cues: {
                      ...config.audio_cues,
                      start_sound: checked ? config.audio_cues.start_sound ?? "sounds/start.wav" : null,
                    },
                  }))
                }
              />
              <Button variant="outline" size="sm" disabled>
                {t("common.edit")}
              </Button>
            </div>
          </SettingRow>

          <SettingRow title={t("recording.soundEnd.title")} description={t("recording.soundEnd.description")}>
            <div className="flex items-center gap-2">
              <Switch
                checked={stopEnabled}
                onCheckedChange={(checked) =>
                  updateDraft((config) => ({
                    ...config,
                    audio_cues: {
                      ...config.audio_cues,
                      stop_sound: checked ? config.audio_cues.stop_sound ?? "sounds/stop.wav" : null,
                    },
                  }))
                }
              />
              <Button variant="outline" size="sm" disabled>
                {t("common.edit")}
              </Button>
            </div>
          </SettingRow>

          <SettingRow title={t("recording.soundError.title")} description={t("recording.soundError.description")}>
            <div className="flex items-center gap-2">
              <Switch
                checked={errorEnabled}
                onCheckedChange={(checked) =>
                  updateDraft((config) => ({
                    ...config,
                    audio_cues: {
                      ...config.audio_cues,
                      error_sound: checked ? config.audio_cues.error_sound ?? "sounds/error_1.wav" : null,
                    },
                  }))
                }
              />
              <Button variant="outline" size="sm" disabled>
                {t("common.edit")}
              </Button>
            </div>
          </SettingRow>

          <SettingRow title={t("recording.autoSwitch.title")} description={t("recording.autoSwitch.description")}>
            <Switch
              checked={draft.audio.auto_switch_to_primary_device}
              onCheckedChange={(checked) =>
                updateDraft((config) => ({
                  ...config,
                  audio: {
                    ...config.audio,
                    auto_switch_to_primary_device: checked,
                  },
                }))
              }
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

function ModelsPane({ draft }: { draft: AppConfig }) {
  const { t } = useTranslation();
  const providerName = useMemo(() => providerLabel(draft.provider.provider, t), [draft.provider.provider, t]);

  return (
    <div className="grid gap-4">
      <Card>
        <CardHeader>
          <CardTitle>{t("models.sttTitle")}</CardTitle>
          <CardDescription>{t("models.sttDescription")}</CardDescription>
        </CardHeader>
        <CardContent className="space-y-3">
          <ProviderItem name={providerName} detail={draft.provider.openrouter.model || t("models.noModel")} />
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
