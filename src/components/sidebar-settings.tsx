import { Info, Mic, Settings, SlidersHorizontal } from "lucide-react";

import { TabsList, TabsTrigger } from "@/components/ui/tabs";

export function SettingsNavigation() {
  return (
    <TabsList className="fixed inset-y-0 left-0 z-10 m-5 h-[calc(100vh-2.5rem)] min-h-[calc(100vh-2.5rem)] justify-start p-3">
      <TabsTrigger value="system" className="h-auto flex-none px-3 py-2">
        <Settings />
        System
      </TabsTrigger>
      <TabsTrigger value="recording" className="h-auto flex-none px-3 py-2">
        <Mic />
        Recording
      </TabsTrigger>
      <TabsTrigger value="models" className="h-auto flex-none px-3 py-2">
        <SlidersHorizontal />
        Models
      </TabsTrigger>
      <TabsTrigger value="about" className="mt-auto h-auto flex-none px-3 py-2">
        <Info />
        About
      </TabsTrigger>
    </TabsList>
  );
}
