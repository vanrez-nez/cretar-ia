import { FileText, Info, Mic, Settings, WandSparkles } from "lucide-react";
import { useTranslation } from "react-i18next";

import { TabsList, TabsTrigger } from "@/components/ui/tabs";

export function SettingsNavigation() {
  const { t } = useTranslation();

  return (
    <TabsList className="fixed inset-y-0 left-0 z-10 m-5 h-[calc(100vh-2.5rem)] min-h-[calc(100vh-2.5rem)] justify-start p-3 bg-card/60">
      <TabsTrigger value="system" className="h-auto flex-none px-3 py-2">
        <Settings />
        {t("sidebar.system")}
      </TabsTrigger>
      <TabsTrigger value="recording" className="h-auto flex-none px-3 py-2">
        <Mic />
        {t("sidebar.recording")}
      </TabsTrigger>
      <TabsTrigger value="transcripts" className="h-auto flex-none px-3 py-2">
        <FileText />
        {t("sidebar.transcripts")}
      </TabsTrigger>
      <TabsTrigger value="transforms" className="h-auto flex-none px-3 py-2">
        <WandSparkles />
        {t("sidebar.transforms")}
      </TabsTrigger>
      <TabsTrigger value="about" className="mt-auto h-auto flex-none px-3 py-2">
        <Info />
        {t("sidebar.about")}
      </TabsTrigger>
    </TabsList>
  );
}
