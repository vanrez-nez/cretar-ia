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
import type { SettingsRecord, SettingValue } from "@/settings/schema";
import { useSettingsStore } from "./stores/settingsStore";
import type { AppLanguage, InteractionMode, PermissionState, PermissionsStatus } from "./lib/types";
import { ShieldCheck, ShieldX } from "lucide-react";

const AUTOSAVE_DELAY_MS = 500;
const APP_VERSION = "0.1.0";
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
    if (!settings) {
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
  draft: SettingsRecord;
  updateDraft: (key: string, value: SettingValue) => void;
};

function SystemPane({ draft, updateDraft }: PaneProps) {
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
            <Switch checked={false} disabled />
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
  const startEnabled = draft["recording.sounds.start"] !== null;
  const stopEnabled = draft["recording.sounds.stop"] !== null;
  const errorEnabled = draft["recording.sounds.error"] !== null;

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
                  updateDraft("recording.sounds.start", checked ? (draft["recording.sounds.start"] ?? "sounds/start.wav") : null)
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
                  updateDraft("recording.sounds.stop", checked ? (draft["recording.sounds.stop"] ?? "sounds/stop.wav") : null)
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
                  updateDraft("recording.sounds.error", checked ? (draft["recording.sounds.error"] ?? "sounds/error_1.wav") : null)
                }
              />
              <Button variant="outline" size="sm" disabled>
                {t("common.edit")}
              </Button>
            </div>
          </SettingRow>

          <SettingRow title={t("recording.autoSwitch.title")} description={t("recording.autoSwitch.description")}>
            <Switch
              checked={draft["recording.microphone.auto_switch_to_primary"] as boolean}
              onCheckedChange={(checked) => updateDraft("recording.microphone.auto_switch_to_primary", checked)}
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
