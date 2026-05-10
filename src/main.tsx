import React from "react";
import { createRoot } from "react-dom/client";

import App from "./App";
import StatusWidgetApp from "./StatusWidgetApp";
import { SystemThemeProvider } from "@/components/system-theme-provider";
import { attachTauriLogger } from "@/lib/logger";
import "./i18n";
import "./main.css";

void attachTauriLogger();

const searchParams = new URLSearchParams(window.location.search);
const isStatusWidget = searchParams.get("window") === "status-widget";
document.documentElement.dataset.window = isStatusWidget ? "status-widget" : "settings";

const RootApp = isStatusWidget ? StatusWidgetApp : App;

createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <SystemThemeProvider>
      <RootApp />
    </SystemThemeProvider>
  </React.StrictMode>,
);
