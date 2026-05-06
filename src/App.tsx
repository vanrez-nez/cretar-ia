import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { isTauri } from "@tauri-apps/api/core";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Separator } from "@/components/ui/separator";
import { Switch } from "@/components/ui/switch";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { HotkeyCapture } from "@/components/hotkey";
import { SettingsBadge } from "@/components/settings-badge";
import { tauriInvoke } from "@/hooks/useTauriIPC";
import { useSettingsStore } from "./stores/settingsStore";
import type { AppConfig, InteractionMode, PermissionState, PermissionsStatus } from "./lib/types";
import { ShieldCheck, ShieldX } from "lucide-react";

const AUTOSAVE_DELAY_MS = 500;
const APP_VERSION = "0.1.0";
export default function App() {
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
    skipNextAutosave.current = true;
    hasHydrated.current = true;
  }, [config]);

  useEffect(() => {
    if (!draftConfig || !hasHydrated.current) {
      return;
    }
    if (skipNextAutosave.current) {
      skipNextAutosave.current = false;
      return;
    }

    const timeout = window.setTimeout(() => {
      void saveSettings(draftConfig).catch((error) => {
        console.error("failed to autosave settings", error);
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
    <main className="min-h-screen bg-background text-foreground">
      <div className="mx-auto flex w-full max-w-5xl flex-col gap-5 p-5">
        <header className="flex flex-col gap-1">
          <h1 className="text-xl font-semibold tracking-tight">Settings</h1>
          <p className="text-sm text-muted-foreground">Changes are saved automatically.</p>
        </header>

        {!draft ? (
          <Card>
            <CardHeader>
              <CardTitle>Loading settings</CardTitle>
              <CardDescription>
                {isTauri() ? "Reading local configuration..." : "Browser preview uses default settings."}
              </CardDescription>
            </CardHeader>
          </Card>
        ) : (
          <Tabs defaultValue="system" className="w-full">
            <TabsList className="grid w-full grid-cols-4">
              <TabsTrigger value="system">System</TabsTrigger>
              <TabsTrigger value="recording">Recording</TabsTrigger>
              <TabsTrigger value="models">Models</TabsTrigger>
              <TabsTrigger value="about">About</TabsTrigger>
            </TabsList>

            <TabsContent value="system" className="mt-4">
              <SystemPane draft={draft} updateDraft={updateDraft} />
            </TabsContent>

            <TabsContent value="recording" className="mt-4">
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

            <TabsContent value="models" className="mt-4">
              <ModelsPane draft={draft} />
            </TabsContent>

            <TabsContent value="about" className="mt-4">
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
  return (
    <div className="grid gap-4 md:grid-cols-[1.2fr_0.8fr]">
      <Card>
        <CardHeader>
          <CardTitle>System</CardTitle>
          <CardDescription>General app behavior and local storage.</CardDescription>
        </CardHeader>
        <CardContent className="space-y-5">
          <SettingRow title="Language" description="UI language support is not wired yet.">
            <Input value="System default" disabled className="max-w-56" />
          </SettingRow>
          <SettingRow title="Launch at start" description="Startup integration is planned for a later phase.">
            <Switch checked={false} disabled />
          </SettingRow>
          <SettingRow title="Save text history" description="Transcript history storage is planned for a later phase.">
            <Switch checked={false} disabled />
          </SettingRow>
          <SettingRow title="Save input audio" description="Keep recordings after successful processing.">
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

      <Card>
        <CardHeader>
          <CardTitle>Overview Stats</CardTitle>
          <CardDescription>Local usage counters are not available yet.</CardDescription>
        </CardHeader>
        <CardContent className="grid gap-3">
          <StatRow label="Transcripts Count" value="0" />
          <StatRow label="Words Transcribed" value="0" />
          <StatRow label="Minutes Recorded" value="0" />
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
  const startEnabled = draft.audio_cues.start_sound !== null;
  const stopEnabled = draft.audio_cues.stop_sound !== null;
  const errorEnabled = draft.audio_cues.error_sound !== null;

  return (
    <div className="grid gap-4">
      <Card>
        <CardHeader>
          <CardTitle>Permissions</CardTitle>
          <CardDescription>
            Refresh checks current macOS permission status and requests missing permissions when available.
          </CardDescription>
        </CardHeader>
        <CardContent className="flex flex-wrap items-center gap-3">
          <PermissionBadge label="Microphone" state={permissions?.microphone} />
          <PermissionBadge label="Accessibility" state={permissions?.accessibility} />
          <Button
            variant="outline"
            size="sm"
            disabled={isRefreshingPermissions}
            onClick={() => void refreshPermissions()}
          >
            {isRefreshingPermissions ? "Refreshing..." : "Refresh"}
          </Button>
          {permissionError ? (
            <span className="text-xs text-destructive">{permissionError}</span>
          ) : null}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Recording</CardTitle>
          <CardDescription>Capture mode, hotkey, microphone, and cues.</CardDescription>
        </CardHeader>
        <CardContent className="space-y-5">
          <SettingRow title="Mode" description="Choose how the hotkey controls recording.">
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
                <SelectItem value="push_to_talk">Push to Talk</SelectItem>
                <SelectItem value="toggle">Toggle</SelectItem>
              </SelectContent>
            </Select>
          </SettingRow>

          <SettingRow title="Hotkey" description="Press modifiers first, then the final trigger key.">
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

          <SettingRow title="Microphone" description="Native device picker is available from the tray for now.">
            <Select disabled value="default">
              <SelectTrigger className="w-64">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="default">Default</SelectItem>
              </SelectContent>
            </Select>
          </SettingRow>

          <Separator />

          <SettingRow title="Sound Record Start" description="Enable or disable the configured start cue.">
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
                Edit
              </Button>
            </div>
          </SettingRow>

          <SettingRow title="Sound Record End" description="Enable or disable the configured stop cue.">
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
                Edit
              </Button>
            </div>
          </SettingRow>

          <SettingRow title="Sound Record Error" description="Enable or disable the configured error cue.">
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
                Edit
              </Button>
            </div>
          </SettingRow>

          <SettingRow title="Auto-switch to primary input device" description="Use the system default input when the selected device is unavailable.">
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

          <SettingRow title="Pause media during recording" description="Media pause integration is planned for a later phase.">
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
  const providerName = useMemo(() => providerLabel(draft.provider.provider), [draft.provider.provider]);

  return (
    <div className="grid gap-4 md:grid-cols-2">
      <Card>
        <CardHeader>
          <CardTitle>STT Model</CardTitle>
          <CardDescription>Speech-to-text providers.</CardDescription>
        </CardHeader>
        <CardContent className="space-y-3">
          <ProviderItem name={providerName} detail={draft.provider.openrouter.model || "No model configured"} />
          <Button variant="outline" size="sm" disabled>
            Add Provider
          </Button>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Formatting Model</CardTitle>
          <CardDescription>Formatting providers are planned for a later phase.</CardDescription>
        </CardHeader>
        <CardContent className="space-y-3">
          <ProviderItem name="No provider configured" detail="Unavailable" disabled />
          <Button variant="outline" size="sm" disabled>
            Add Provider
          </Button>
        </CardContent>
      </Card>
    </div>
  );
}

function AboutPane() {
  return (
    <Card>
      <CardHeader>
        <CardTitle>About</CardTitle>
        <CardDescription>Application information and project links.</CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        <SettingRow title="Version Info" description="Current app version.">
          <Badge variant="outline">{APP_VERSION}</Badge>
        </SettingRow>
        <SettingRow title="Website" description="Product website link.">
          <Button variant="link" size="sm" disabled>
            Website
          </Button>
        </SettingRow>
        <SettingRow title="GitHub Link" description="Repository link.">
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
    <div className="grid gap-3 rounded-lg border border-border/70 bg-card/50 p-3 sm:grid-cols-[1fr_auto] sm:items-center">
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
    <div className="flex items-center justify-between rounded-lg border border-border/70 bg-card/50 px-3 py-2">
      <span className="text-sm text-muted-foreground">{label}</span>
      <span className="text-lg font-semibold tabular-nums">{value}</span>
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
  return (
    <div className="flex items-center justify-between gap-3 rounded-lg border border-border/70 bg-card/50 p-3 opacity-100 data-[disabled=true]:opacity-50" data-disabled={disabled}>
      <div className="min-w-0">
        <div className="truncate text-sm font-medium">{name}</div>
        <div className="truncate text-xs text-muted-foreground">{detail}</div>
      </div>
      <div className="flex shrink-0 gap-2">
        <Button variant="outline" size="sm" disabled>
          Edit
        </Button>
        <Button variant="ghost" size="sm" disabled>
          Remove
        </Button>
      </div>
    </div>
  );
}

function providerLabel(provider: string) {
  if (provider === "openrouter") {
    return "OpenRouter";
  }
  return provider || "Provider";
}
