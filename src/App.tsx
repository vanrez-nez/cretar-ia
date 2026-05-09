import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { isTauri } from "@tauri-apps/api/core";
import { confirm, open, save } from "@tauri-apps/plugin-dialog";
import { useTranslation } from "react-i18next";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardAction, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Separator } from "@/components/ui/separator";
import { Switch } from "@/components/ui/switch";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import {
  Pagination,
  PaginationContent,
  PaginationEllipsis,
  PaginationItem,
  PaginationLink,
  PaginationNext,
  PaginationPrevious,
} from "@/components/ui/pagination";
import { SettingsNavigation } from "@/components/sidebar-settings";
import { HotkeyCapture } from "@/components/hotkey";
import { JsonTextarea } from "@/components/json-textarea";
import { SettingsBadge } from "@/components/settings-badge";
import { SettingsItemCard } from "@/components/settings-item-card";
import { SettingsItemEditor } from "@/components/settings-item-editor";
import { AudioMiniPlayer, type AudioWaveform } from "@/components/audio-mini-player";
import { tauriInvoke, useTauriEvent } from "@/hooks/useTauriIPC";
import i18n, { resolveAppLocale } from "@/i18n";
import { logger } from "@/lib/logger";
import { setLaunchAtStart } from "@/settings/autostart";
import type { SettingsRecord, SettingValue } from "@/settings/schema";
import { useSettingsStore } from "./stores/settingsStore";
import type { AppLanguage, InteractionMode, PermissionState, PermissionsStatus } from "./lib/types";
import { Check, Download, Eye, MoreHorizontal, Play, Plus, RefreshCw, ShieldCheck, ShieldX, SquarePen, Trash2, TriangleAlert } from "lucide-react";

const AUTOSAVE_DELAY_MS = 500;
const APP_VERSION = "0.1.0";
const SYSTEM_DEFAULT_INPUT_DEVICE = "__system_default__";
const HISTORY_PAGE_SIZE = 7;

type SettingsChangedEvent = {
  source: string;
  keys: string[];
};

type HistoryOverview = {
  transcripts: number;
  words: number;
  minutes: number;
};

type HistoryRecord = {
  id: string;
  audioFilePath: string | null;
  audioDurationMs: number;
  transcriptText: string | null;
  transformText: string | null;
  errorMessage: string | null;
  createdAt: string;
};

type HistoryPage = {
  items: HistoryRecord[];
  page: number;
  pageSize: number;
  total: number;
};

type HistoryExportStatus = "idle" | "packing" | "complete" | "saving" | "saved" | "error";

type HistoryExportState = {
  exportId: string | null;
  status: HistoryExportStatus;
  processed: number;
  total: number;
  message: string | null;
  savedPath: string | null;
};

type HistoryExportStart = {
  exportId: string;
};

type HistoryExportProgressEvent = {
  exportId: string;
  status: "packing" | "complete" | "error";
  processed: number;
  total: number;
  message?: string | null;
};

type HistoryExportSaveResult = {
  path: string;
};

type SoundOption = {
  id: string;
  label: string;
  file: string;
};

type SoundSlot = "start" | "stop" | "error";

const IDLE_HISTORY_EXPORT: HistoryExportState = {
  exportId: null,
  status: "idle",
  processed: 0,
  total: 0,
  message: null,
  savedPath: null,
};

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
  const [historyOverview, setHistoryOverview] = useState<HistoryOverview>({
    transcripts: 0,
    words: 0,
    minutes: 0,
  });
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

  const refreshHistoryOverview = useCallback(async () => {
    if (!isTauri()) {
      setHistoryOverview({ transcripts: 0, words: 0, minutes: 0 });
      return;
    }
    setHistoryOverview(await tauriInvoke<HistoryOverview>("get_history_overview"));
  }, []);

  useEffect(() => {
    void refreshHistoryOverview().catch((error) => {
      logger.warn("Failed to refresh history overview", error);
    });
  }, [refreshHistoryOverview]);

  useTauriEvent<SettingsChangedEvent>("settings:changed", (event) => {
    if (event.keys.includes("history.overview")) {
      void refreshHistoryOverview().catch((error) => {
        logger.warn("Failed to refresh history overview after change", error);
      });
    }
  });

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
                historyOverview={historyOverview}
              />
            </TabsContent>

            <TabsContent value="history" className="h-screen overflow-x-hidden overflow-y-auto overscroll-contain p-5">
              <HistoryPane />
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
  historyOverview,
}: PaneProps & {
  isUpdatingAutostart: boolean;
  updateLaunchAtStart: (checked: boolean) => Promise<void>;
  historyOverview: HistoryOverview;
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
        </CardContent>
      </Card>

      <Card className="h-full">
        <CardHeader>
          <CardTitle>{t("system.stats.title")}</CardTitle>
        </CardHeader>
        <CardContent className="grid grid-cols-3 items-start gap-3">
          <StatRow label={t("system.stats.transcripts")} value={String(historyOverview.transcripts)} />
          <StatRow label={t("system.stats.words")} value={String(historyOverview.words)} />
          <StatRow label={t("system.stats.minutes")} value={String(historyOverview.minutes)} />
        </CardContent>
      </Card>
    </div>
  );
}

function HistoryPane() {
  const { t } = useTranslation();
  const settings = useSettingsStore((s) => s.settings);
  const saveSettings = useSettingsStore((s) => s.updateSettings);
  const isSavingSettings = useSettingsStore((s) => s.isSaving);
  const [historyPage, setHistoryPage] = useState<HistoryPage | null>(null);
  const [page, setPage] = useState(1);
  const [selectedRecord, setSelectedRecord] = useState<HistoryRecord | null>(null);
  const [waveform, setWaveform] = useState<AudioWaveform | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [isWaveformLoading, setIsWaveformLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [exportState, setExportState] = useState<HistoryExportState>(IDLE_HISTORY_EXPORT);

  const loadWaveform = useCallback(async (record: HistoryRecord | null) => {
    setWaveform(null);
    if (!record?.audioFilePath) {
      return;
    }

    setIsWaveformLoading(true);
    try {
      setWaveform(
        await tauriInvoke<AudioWaveform>("get_history_audio_waveform", {
          historyId: record.id,
          samples: 1024,
        })
      );
    } catch (error) {
      logger.warn("Failed to load history waveform", {
        id: record.id,
        error: error instanceof Error ? error.message : String(error),
      });
      setWaveform(null);
    } finally {
      setIsWaveformLoading(false);
    }
  }, []);

  const load = useCallback(async () => {
    if (!isTauri()) {
      setHistoryPage(null);
      setSelectedRecord(null);
      setWaveform(null);
      return;
    }

    setIsLoading(true);
    setError(null);
    try {
      const nextPage = await tauriInvoke<HistoryPage>("list_history_records", {
        page,
        pageSize: HISTORY_PAGE_SIZE,
      });
      setHistoryPage(nextPage);
      const nextSelected = nextPage.items.find((record) => record.audioFilePath) ?? nextPage.items[0] ?? null;
      setSelectedRecord(nextSelected);
      await loadWaveform(nextSelected);
    } catch (error) {
      setError(error instanceof Error ? error.message : String(error));
      setHistoryPage(null);
      setSelectedRecord(null);
      setWaveform(null);
    } finally {
      setIsLoading(false);
    }
  }, [loadWaveform, page]);

  useEffect(() => {
    void load();
  }, [load]);

  useTauriEvent<SettingsChangedEvent>("settings:changed", (event) => {
    if (event.keys.includes("history.overview")) {
      void load();
    }
  });

  useTauriEvent<HistoryExportProgressEvent>("history:export-progress", (event) => {
    setExportState((current) => {
      if (current.exportId && current.exportId !== event.exportId) {
        return current;
      }

      return {
        exportId: event.exportId,
        status: event.status,
        processed: event.processed,
        total: event.total,
        message: event.message ?? null,
        savedPath: null,
      };
    });
  });

  const selectRecord = useCallback((record: HistoryRecord) => {
    setSelectedRecord(record);
    void loadWaveform(record);
  }, [loadWaveform]);

  const items = historyPage?.items ?? [];
  const total = historyPage?.total ?? 0;
  const totalPages = Math.max(1, Math.ceil(total / HISTORY_PAGE_SIZE));
  const paginationItems = historyPaginationItems(page, totalPages);
  const selectedText = selectedRecord ? historyRecordText(selectedRecord) : null;
  const canSaveSelectedRecord = Boolean(selectedRecord?.audioFilePath && waveform);
  const historyEnabled = Boolean(settings?.["system.history_enabled"]);
  const hasPendingExport =
    exportState.status === "packing" ||
    exportState.status === "complete" ||
    exportState.status === "saving";

  const updateHistoryEnabled = useCallback((enabled: boolean) => {
    if (!settings) {
      return;
    }

    setActionError(null);
    void saveSettings({
      ...settings,
      "system.history_enabled": enabled,
    }).catch((error) => {
      const message = error instanceof Error ? error.message : String(error);
      setActionError(message);
    });
  }, [saveSettings, settings]);

  const startHistoryExport = useCallback(async () => {
    if (hasPendingExport) {
      return;
    }

    setActionError(null);
    setExportState({
      ...IDLE_HISTORY_EXPORT,
      status: "packing",
      message: t("history.exportPreparing"),
    });

    try {
      const result = await tauriInvoke<HistoryExportStart>("start_history_export");
      setExportState((current) => {
        if (current.exportId === result.exportId) {
          return current;
        }

        return {
          ...current,
          exportId: result.exportId,
          status: current.status === "idle" ? "packing" : current.status,
        };
      });
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setExportState({
        ...IDLE_HISTORY_EXPORT,
        status: "error",
        message,
      });
    }
  }, [hasPendingExport, t]);

  const saveHistoryExport = useCallback(async () => {
    if (!exportState.exportId || exportState.status !== "complete") {
      return;
    }

    const destinationPath = await save({
      title: t("history.exportSaveTitle"),
      defaultPath: `history-export-${new Date().toISOString().slice(0, 10)}.zip`,
      filters: [{ name: "ZIP", extensions: ["zip"] }],
    });

    if (!destinationPath) {
      return;
    }

    setExportState((current) => ({
      ...current,
      status: "saving",
      message: t("history.exportSaving"),
    }));

    try {
      const result = await tauriInvoke<HistoryExportSaveResult>("save_history_export", {
        exportId: exportState.exportId,
        destinationPath,
      });
      setExportState((current) => ({
        ...current,
        status: "saved",
        message: t("history.exportSaved"),
        savedPath: result.path,
      }));
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setExportState((current) => ({
        ...current,
        status: "complete",
        message: `${t("history.exportSaveError")}: ${message}`,
      }));
    }
  }, [exportState.exportId, exportState.status, t]);

  const dismissHistoryExport = useCallback(() => {
    const exportId = exportState.exportId;
    const shouldCleanup = exportId && exportState.status !== "saved";
    setExportState(IDLE_HISTORY_EXPORT);
    if (shouldCleanup) {
      void tauriInvoke<void>("delete_history_export", { exportId }).catch((error) => {
        logger.warn("failed to delete temporary history export", {
          exportId,
          error: error instanceof Error ? error.message : String(error),
        });
      });
    }
  }, [exportState.exportId, exportState.status]);

  const retryHistoryExport = useCallback(() => {
    const exportId = exportState.exportId;
    if (exportId) {
      void tauriInvoke<void>("delete_history_export", { exportId }).catch((error) => {
        logger.warn("failed to delete failed history export", {
          exportId,
          error: error instanceof Error ? error.message : String(error),
        });
      });
    }
    void startHistoryExport();
  }, [exportState.exportId, startHistoryExport]);

  const deleteAllHistory = useCallback(async () => {
    const confirmed = await confirm(t("history.deleteAllConfirmDescription"), {
      title: t("history.deleteAllConfirmTitle"),
      kind: "warning",
      okLabel: t("history.deleteAll"),
      cancelLabel: t("common.cancel"),
    });

    if (!confirmed) {
      return;
    }

    setActionError(null);
    try {
      await tauriInvoke("delete_all_history");
      setPage(1);
      setHistoryPage({
        items: [],
        page: 1,
        pageSize: HISTORY_PAGE_SIZE,
        total: 0,
      });
      setSelectedRecord(null);
      setWaveform(null);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setActionError(message);
    }
  }, [t]);

  const deleteSelectedRecord = useCallback(async () => {
    if (!selectedRecord) {
      return;
    }

    const confirmed = await confirm(t("history.deleteRecordConfirmDescription"), {
      title: t("history.deleteRecordConfirmTitle"),
      kind: "warning",
      okLabel: t("history.deleteRecord"),
      cancelLabel: t("common.cancel"),
    });

    if (!confirmed) {
      return;
    }

    setActionError(null);
    try {
      await tauriInvoke("delete_history_record", { historyId: selectedRecord.id });
      setSelectedRecord(null);
      setWaveform(null);
      if (items.length <= 1 && page > 1) {
        setPage((current) => Math.max(1, current - 1));
      } else {
        void load();
      }
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setActionError(`${t("history.deleteRecordError")}: ${message}`);
    }
  }, [items.length, load, page, selectedRecord, t]);

  const saveSelectedRecord = useCallback(async () => {
    if (!selectedRecord?.audioFilePath) {
      return;
    }

    const destinationPath = await save({
      title: t("history.saveRecordTitle"),
      defaultPath: `transcript-${new Date().toISOString().slice(0, 10)}.zip`,
      filters: [{ name: "ZIP", extensions: ["zip"] }],
    });

    if (!destinationPath) {
      return;
    }

    setActionError(null);
    try {
      await tauriInvoke<HistoryExportSaveResult>("save_history_record_export", {
        historyId: selectedRecord.id,
        destinationPath,
      });
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setActionError(`${t("history.saveRecordError")}: ${message}`);
    }
  }, [selectedRecord, t]);

  return (
    <div className="grid gap-4">
      {exportState.status !== "idle" ? (
        <HistoryExportProgressCard
          state={exportState}
          onDismiss={dismissHistoryExport}
          onRetry={retryHistoryExport}
          onSave={saveHistoryExport}
        />
      ) : null}

      <Card>
        <CardHeader>
          <CardTitle>{t("history.title")}</CardTitle>
          <CardDescription>{t("history.description")}</CardDescription>
          <CardAction>
            <ContextMenu>
              <ContextMenuTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon-sm"
                  aria-label={t("history.menuLabel")}
                  onClick={(event) => {
                    event.preventDefault();
                    const rect = event.currentTarget.getBoundingClientRect();
                    event.currentTarget.dispatchEvent(
                      new MouseEvent("contextmenu", {
                        bubbles: true,
                        cancelable: true,
                        clientX: event.clientX || rect.right,
                        clientY: event.clientY || rect.bottom,
                      })
                    );
                  }}
                >
                  <MoreHorizontal aria-hidden="true" />
                </Button>
              </ContextMenuTrigger>
              <ContextMenuContent>
                <ContextMenuItem
                  disabled={hasPendingExport}
                  onSelect={() => void startHistoryExport()}
                >
                  {t("history.export")}
                </ContextMenuItem>
                <ContextMenuItem
                  className="text-destructive focus:text-destructive"
                  onSelect={() => void deleteAllHistory()}
                >
                  {t("history.deleteAll")}
                </ContextMenuItem>
                <ContextMenuSeparator />
                <ContextMenuItem
                  className="justify-between gap-6"
                  disabled={!settings || isSavingSettings}
                  onSelect={(event) => {
                    event.preventDefault();
                    updateHistoryEnabled(!historyEnabled);
                  }}
                >
                  <span>{t("history.enable")}</span>
                  <Switch
                    size="sm"
                    checked={historyEnabled}
                    disabled={!settings || isSavingSettings}
                    onClick={(event) => event.stopPropagation()}
                    onCheckedChange={updateHistoryEnabled}
                  />
                </ContextMenuItem>
              </ContextMenuContent>
            </ContextMenu>
          </CardAction>
        </CardHeader>
        <CardContent className="space-y-3">
          <Table className="table-fixed">
            <TableHeader>
              <TableRow>
                <TableHead className="w-44">{t("history.time")}</TableHead>
                <TableHead className="w-24">{t("history.duration")}</TableHead>
                <TableHead>{t("history.text")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {items.map((record) => {
                const isSelected = selectedRecord?.id === record.id;
                return (
                  <TableRow
                    key={record.id}
                    data-state={isSelected ? "selected" : undefined}
                    onClick={() => selectRecord(record)}
                  >
                    <TableCell>{formatDateTime(record.createdAt)}</TableCell>
                    <TableCell>{formatDurationMs(record.audioDurationMs)}</TableCell>
                    <TableCell className="max-w-0 overflow-hidden">
                      <div className="truncate">
                        {historyRecordText(record) || t("common.unavailable")}
                      </div>
                    </TableCell>
                  </TableRow>
                );
              })}
            </TableBody>
          </Table>
          {!isLoading && items.length === 0 ? (
            <div className="rounded-lg bg-muted/40 p-3 text-sm text-muted-foreground">
              {t("history.empty")}
            </div>
          ) : null}
          {isLoading ? (
            <div className="rounded-lg bg-muted/40 p-3 text-sm text-muted-foreground">
              {t("history.loading")}
            </div>
          ) : null}
          {actionError ? <p className="text-xs text-destructive">{actionError}</p> : null}
          {error ? <p className="text-xs text-destructive">{t("history.loadError")}: {error}</p> : null}
          <Pagination className="justify-end">
            <PaginationContent>
              <PaginationItem>
                <PaginationPrevious
                  disabled={page <= 1}
                  href="#"
                  onClick={(event) => {
                    event.preventDefault();
                    if (page > 1) {
                      setPage((current) => Math.max(1, current - 1));
                    }
                  }}
                />
              </PaginationItem>
              {paginationItems.map((item, index) => (
                <PaginationItem key={`${item}-${index}`}>
                  {item === "ellipsis" ? (
                    <PaginationEllipsis />
                  ) : (
                    <PaginationLink
                      href="#"
                      isActive={item === page}
                      onClick={(event) => {
                        event.preventDefault();
                        setPage(item);
                      }}
                    >
                      {item}
                    </PaginationLink>
                  )}
                </PaginationItem>
              ))}
              <PaginationItem>
                <PaginationNext
                  disabled={page >= totalPages || totalPages <= 0}
                  href="#"
                  onClick={(event) => {
                    event.preventDefault();
                    if (page < totalPages && totalPages > 0) {
                      setPage((current) => Math.min(totalPages, current + 1));
                    }
                  }}
                />
              </PaginationItem>
            </PaginationContent>
          </Pagination>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>{t("history.transcript")}</CardTitle>
          {selectedRecord ? (
            <CardDescription>{formatRelativeTime(selectedRecord.createdAt, i18n.resolvedLanguage)}</CardDescription>
          ) : null}
          <CardAction className="flex gap-1">
            <Button
              variant="ghost"
              size="sm"
              className="text-destructive hover:text-destructive"
              disabled={!selectedRecord}
              onClick={() => void deleteSelectedRecord()}
            >
              <Trash2 aria-hidden="true" />
              {t("history.deleteRecord")}
            </Button>
            <Button
              variant="ghost"
              size="sm"
              disabled={!canSaveSelectedRecord}
              onClick={() => void saveSelectedRecord()}
            >
              <Download aria-hidden="true" />
              {t("history.saveRecord")}
            </Button>
          </CardAction>
        </CardHeader>
        <CardContent className="space-y-3 pb-4">
          {!isLoading && !isWaveformLoading && !selectedRecord ? (
            <div className="rounded-lg bg-muted/40 p-3 text-sm text-muted-foreground">
              {t("history.selectRecord")}
            </div>
          ) : null}
          {selectedRecord?.audioFilePath && waveform ? (
            <AudioMiniPlayer audioFilePath={selectedRecord.audioFilePath} waveform={waveform} />
          ) : null}
          {selectedText ? (
            <p className="select-text whitespace-pre-wrap break-words text-sm text-muted-foreground">{selectedText}</p>
          ) : null}
          {isWaveformLoading ? (
            <div className="rounded-lg bg-muted/40 p-3 text-sm text-muted-foreground">
              {t("history.loadingAudio")}
            </div>
          ) : null}
          {!isWaveformLoading && selectedRecord && !selectedRecord.audioFilePath ? (
            <div className="rounded-lg bg-muted/40 p-3 text-sm text-muted-foreground">
              {t("history.audioUnavailable")}
            </div>
          ) : null}
          {!isWaveformLoading && selectedRecord?.audioFilePath && !waveform ? (
            <div className="rounded-lg bg-muted/40 p-3 text-sm text-muted-foreground">
              {t("history.audioUnavailable")}
            </div>
          ) : null}
        </CardContent>
      </Card>
    </div>
  );
}

function HistoryExportProgressCard({
  state,
  onDismiss,
  onRetry,
  onSave,
}: {
  state: HistoryExportState;
  onDismiss: () => void;
  onRetry: () => void;
  onSave: () => void;
}) {
  const { t } = useTranslation();
  const percent = historyExportPercent(state);
  const message = state.message ?? historyExportMessage(t, state.status);

  return (
    <Card>
      <CardHeader>
        <CardTitle>{t("history.exportTitle")}</CardTitle>
        <CardDescription>{message}</CardDescription>
      </CardHeader>
      <CardContent className="space-y-3">
        <div className="space-y-1.5">
          <div className="h-1.5 overflow-hidden rounded-full bg-muted">
            <div
              className="h-full rounded-full bg-primary transition-[width]"
              style={{ width: `${percent}%` }}
            />
          </div>
          <div className="flex items-center justify-between text-xs text-muted-foreground">
            <span>
              {t("history.exportProgress", {
                processed: state.processed,
                total: state.total,
                percent,
              })}
            </span>
            {state.savedPath ? <span className="truncate pl-3">{state.savedPath}</span> : null}
          </div>
        </div>
        <div className="flex justify-end gap-2">
          {state.status === "error" ? (
            <Button variant="ghost" size="sm" onClick={onRetry}>
              {t("history.exportRetry")}
            </Button>
          ) : null}
          {state.status === "complete" ? (
            <Button size="sm" onClick={onSave}>
              {t("history.exportSave")}
            </Button>
          ) : null}
          {state.status === "saving" ? (
            <Button size="sm" disabled>
              {t("history.exportSaving")}
            </Button>
          ) : null}
          <Button
            variant="ghost"
            size="sm"
            disabled={state.status === "packing" || state.status === "saving"}
            onClick={onDismiss}
          >
            {t("history.exportDismiss")}
          </Button>
        </div>
      </CardContent>
    </Card>
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

type PromptView = {
  id: string;
  name: string;
  description: string;
  template: string;
  is_active: boolean;
  is_preset: boolean;
  created_at: string;
  updated_at: string;
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
      {isFormatting ? <PromptsCard transformEnabled={transformEnabled} /> : null}
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

function PromptsCard({ transformEnabled }: { transformEnabled?: boolean }) {
  const { t } = useTranslation();
  const [prompts, setPrompts] = useState<PromptView[]>([]);
  const [isAdding, setIsAdding] = useState(false);
  const [editingPromptId, setEditingPromptId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const isDisabled = transformEnabled === false;

  const load = useCallback(async () => {
    if (!isTauri()) {
      return;
    }
    setError(null);
    try {
      setPrompts(await tauriInvoke<PromptView[]>("list_prompts"));
    } catch (error) {
      setError(error instanceof Error ? error.message : String(error));
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  useTauriEvent<SettingsChangedEvent>("settings:changed", (event) => {
    if (event.keys.includes("prompts.list") || event.keys.includes("prompts.active")) {
      void load();
    }
  });

  const handleSaved = () => {
    setIsAdding(false);
    setEditingPromptId(null);
    void load();
  };

  return (
    <Card>
      <CardHeader className="flex flex-row items-start justify-between gap-4">
        <div className="min-w-0 select-none space-y-1.5">
          <CardTitle>{t("prompts.cardTitle")}</CardTitle>
          <CardDescription>{t("prompts.description")}</CardDescription>
        </div>
      </CardHeader>
      <CardContent className={`space-y-3 ${isDisabled ? "opacity-50" : ""}`}>
        {prompts.length === 0 && !isAdding ? (
          <div className="rounded-lg bg-muted/40 p-3 text-sm text-muted-foreground">
            {t("prompts.noPrompt")}
          </div>
        ) : null}
        {prompts.map((prompt) => (
          <PromptItem
            key={prompt.id}
            prompt={prompt}
            isLocked={isDisabled || (editingPromptId !== null && editingPromptId !== prompt.id)}
            onSaved={handleSaved}
            onSelected={() => void load()}
            onEditingChange={(editing) => setEditingPromptId(editing ? prompt.id : null)}
            onDeleted={() => void load()}
          />
        ))}
        {isAdding ? (
          <PromptItem
            prompt={null}
            isLocked={isDisabled || (editingPromptId !== null && editingPromptId !== "__new__")}
            onSaved={handleSaved}
            onSelected={() => void load()}
            onEditingChange={(editing) => setEditingPromptId(editing ? "__new__" : null)}
            onCancel={() => {
              setEditingPromptId(null);
              setIsAdding(false);
            }}
          />
        ) : (
          <div className="flex justify-end">
            <Button
              variant="outline"
              size="sm"
              disabled={isDisabled || editingPromptId !== null}
              onClick={() => {
                setEditingPromptId("__new__");
                setIsAdding(true);
              }}
            >
              <Plus className="size-4" aria-hidden="true" />
              {t("prompts.addPrompt")}
            </Button>
          </div>
        )}
        {error ? <p className="text-xs text-destructive">{error}</p> : null}
      </CardContent>
    </Card>
  );
}

function PromptItem({
  prompt,
  isLocked,
  onSaved,
  onSelected,
  onEditingChange,
  onDeleted,
  onCancel,
}: {
  prompt: PromptView | null;
  isLocked: boolean;
  onSaved: () => void;
  onSelected: () => void;
  onEditingChange: (editing: boolean) => void;
  onDeleted?: () => void;
  onCancel?: () => void;
}) {
  const { t } = useTranslation();
  const [isEditing, setIsEditing] = useState(prompt === null);
  const [name, setName] = useState(prompt?.name ?? "");
  const [description, setDescription] = useState(prompt?.description ?? "");
  const [template, setTemplate] = useState(prompt?.template ?? "");
  const [isSaving, setIsSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const isPreset = prompt?.is_preset ?? false;
  const isReadOnly = Boolean(prompt && isPreset);

  const save = async () => {
    setIsSaving(true);
    setError(null);
    try {
      await tauriInvoke<string>("save_prompt", {
        promptId: prompt?.id ?? null,
        name,
        description,
        template,
      });
      setIsEditing(false);
      onEditingChange(false);
      onSaved();
    } catch (error) {
      setError(`${t("prompts.saveError")}: ${error instanceof Error ? error.message : String(error)}`);
    } finally {
      setIsSaving(false);
    }
  };

  const remove = async () => {
    if (!prompt || isLocked || isPreset) {
      return;
    }
    setIsSaving(true);
    setError(null);
    try {
      await tauriInvoke<void>("delete_prompt", { promptId: prompt.id });
      onDeleted?.();
    } catch (error) {
      setError(`${t("prompts.saveError")}: ${error instanceof Error ? error.message : String(error)}`);
    } finally {
      setIsSaving(false);
    }
  };

  const select = async () => {
    if (!prompt || prompt.is_active || isLocked) {
      return;
    }
    setIsSaving(true);
    setError(null);
    try {
      await tauriInvoke<void>("select_prompt", { promptId: prompt.id });
      onSelected();
    } catch (error) {
      setError(`${t("prompts.saveError")}: ${error instanceof Error ? error.message : String(error)}`);
    } finally {
      setIsSaving(false);
    }
  };

  const cancel = () => {
    setIsEditing(false);
    onEditingChange(false);
    onCancel?.();
  };

  if (!isEditing && prompt) {
    return (
      <SettingsItemCard
        isLocked={isLocked}
        isSelected={prompt.is_active}
        onSelect={() => void select()}
        actions={
          isPreset ? (
            <Button
              variant="ghost"
              size="icon"
              aria-label={t("common.view")}
              title={t("common.view")}
              onClick={(event) => {
                event.stopPropagation();
                setIsEditing(true);
                onEditingChange(true);
              }}
            >
              <Eye className="size-4" aria-hidden="true" />
            </Button>
          ) : (
            <>
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
            </>
          )
        }
        status={prompt.is_active ? <Check className="size-4 text-emerald-500" aria-hidden="true" /> : null}
      >
          <div className="flex min-w-0 items-center gap-2">
            <div className="truncate text-sm font-medium">{prompt.name}</div>
          </div>
          <div className="truncate text-xs text-muted-foreground">{prompt.description}</div>
      </SettingsItemCard>
    );
  }

  return (
    <SettingsItemEditor isLocked={isLocked}>
      <div className="grid gap-3 sm:grid-cols-2">
        <div className="space-y-1.5">
          <Label>{t("prompts.name")}</Label>
          <Input value={name} disabled={isReadOnly || isLocked || isSaving} onChange={(event) => setName(event.target.value)} />
        </div>
        <div className="space-y-1.5">
          <Label>{t("prompts.descriptionField")}</Label>
          <Input
            value={description}
            disabled={isReadOnly || isLocked || isSaving}
            onChange={(event) => setDescription(event.target.value)}
          />
        </div>
      </div>
      <div className="space-y-1.5">
        <Label>{t("prompts.template")}</Label>
        <textarea
          value={template}
          disabled={isReadOnly || isLocked || isSaving}
          onChange={(event) => setTemplate(event.target.value)}
          className="min-h-32 w-full resize-y rounded-md border border-input bg-background px-3 py-2 text-sm text-foreground shadow-sm outline-none transition-colors placeholder:text-muted-foreground focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50"
        />
      </div>
      <div className="flex justify-end gap-2">
        <Button variant="ghost" size="sm" disabled={isSaving} onClick={cancel}>
          {t("common.cancel")}
        </Button>
        {isReadOnly ? null : (
          <Button size="sm" disabled={isLocked || isSaving} onClick={() => void save()}>
            {t("common.save")}
          </Button>
        )}
      </div>
      {error ? <p className="text-xs text-destructive">{error}</p> : null}
    </SettingsItemEditor>
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
  const [showAdvancedConfig, setShowAdvancedConfig] = useState(false);
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
    const nextValue = path.join(".") === "auth.api_key" ? value.trim() : value;
    updateProviderConfig(setNestedValue(source, path, nextValue));
  };

  const changeProvider = (nextProviderId: string) => {
    setProviderId(nextProviderId);
    const provider = providers.find((item) => item.id === nextProviderId);
    const nextOptions = modelOptionsForProvider(nextProviderId);
    const firstOption = nextOptions[0];
    const firstCatalogModel = firstOption ? catalogModelById(firstOption.id) : undefined;
    setProviderConfigText(formatJson(provider?.config ?? {}));
    setSelectedCatalogModelId(firstOption?.id ?? "");
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
        userModelId: model?.id ?? null,
        role,
        providerId,
        modelId: selectedCatalogModelId,
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
      <SettingsItemCard
        isLocked={isLocked}
        isSelected={isSelected}
        onSelect={() => void select()}
        actions={
          <>
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
          </>
        }
        status={<ModelHealthIcon health={health} isSelected={isSelected} />}
      >
          <div className="flex min-w-0 items-center gap-2">
            <div className="truncate text-sm font-medium">{model.provider_name}</div>
          </div>
          <div className="truncate text-xs text-muted-foreground">
            {model.model_display_name || model.external_model_id}
          </div>
      </SettingsItemCard>
    );
  }

  return (
    <SettingsItemEditor>
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
            autoComplete="off"
            onChange={(event) => updateProviderField(["auth", "api_key"], event.target.value)}
          />
        </div>
      ) : null}

      {showAdvancedConfig ? (
      <div className="space-y-3">
          <Tabs defaultValue="provider" orientation="horizontal" className="advanced-json-tabs w-full">
            <TabsList variant="line" className="advanced-json-tabs-list">
              <TabsTrigger value="provider" className="advanced-json-tabs-trigger text-xs">
                {t("models.provider")}
              </TabsTrigger>
              <TabsTrigger value="model" className="advanced-json-tabs-trigger text-xs">
                {t("models.model")}
              </TabsTrigger>
            </TabsList>
            <TabsContent value="provider" className="pt-3">
              <JsonTextarea
                value={providerConfigText}
                onChange={setProviderConfigText}
              />
            </TabsContent>
            <TabsContent value="model" className="pt-3">
              <JsonTextarea
                value={modelConfigText}
                onChange={setModelConfigText}
              />
            </TabsContent>
          </Tabs>
      </div>
      ) : null}

      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          {!showAdvancedConfig ? (
            <Button
              type="button"
              variant="ghost"
              size="sm"
              className="px-2"
              onClick={() => setShowAdvancedConfig(true)}
            >
              {t("models.advanced")}
            </Button>
          ) : null}
        </div>
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
    </SettingsItemEditor>
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

function formatDateTime(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return value;
  }
  return date.toLocaleString();
}

function formatRelativeTime(value: string, locale?: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return value;
  }

  const formatter = new Intl.RelativeTimeFormat(locale, { numeric: "auto" });
  const divisions: Array<{ amount: number; unit: Intl.RelativeTimeFormatUnit }> = [
    { amount: 60, unit: "second" },
    { amount: 60, unit: "minute" },
    { amount: 24, unit: "hour" },
    { amount: 7, unit: "day" },
    { amount: 4.34524, unit: "week" },
    { amount: 12, unit: "month" },
    { amount: Number.POSITIVE_INFINITY, unit: "year" },
  ];

  let duration = (date.getTime() - Date.now()) / 1000;
  for (const division of divisions) {
    if (Math.abs(duration) < division.amount) {
      return formatter.format(Math.round(duration), division.unit);
    }
    duration /= division.amount;
  }

  return formatDateTime(value);
}

function formatDurationMs(value: number): string {
  if (!Number.isFinite(value) || value <= 0) {
    return "0:00";
  }
  const totalSeconds = Math.round(value / 1000);
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return `${minutes}:${String(seconds).padStart(2, "0")}`;
}

function historyRecordText(record: HistoryRecord): string {
  return record.transformText ?? record.transcriptText ?? record.errorMessage ?? "";
}

function historyExportPercent(state: HistoryExportState): number {
  if (state.status === "complete" || state.status === "saving" || state.status === "saved") {
    return 100;
  }
  if (state.total <= 0) {
    return 0;
  }
  return Math.min(100, Math.max(0, Math.round((state.processed / state.total) * 100)));
}

function historyExportMessage(t: ReturnType<typeof useTranslation>["t"], status: HistoryExportStatus): string {
  switch (status) {
    case "packing":
      return t("history.exportPacking");
    case "complete":
      return t("history.exportReady");
    case "saving":
      return t("history.exportSaving");
    case "saved":
      return t("history.exportSaved");
    case "error":
      return t("history.exportFailed");
    case "idle":
    default:
      return "";
  }
}

function historyPaginationItems(page: number, totalPages: number): Array<number | "ellipsis"> {
  if (totalPages <= 5) {
    return Array.from({ length: totalPages }, (_, index) => index + 1);
  }

  const visible = new Set([1, totalPages, page - 1, page, page + 1].filter((item) => item >= 1 && item <= totalPages));
  const sorted = Array.from(visible).sort((a, b) => a - b);
  const items: Array<number | "ellipsis"> = [];

  sorted.forEach((item, index) => {
    const previous = sorted[index - 1];
    if (previous !== undefined && item - previous > 1) {
      items.push("ellipsis");
    }
    items.push(item);
  });

  return items;
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
