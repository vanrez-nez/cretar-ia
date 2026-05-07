import React from "react";
import { createRoot } from "react-dom/client";

import App from "./App";
import { SystemThemeProvider } from "@/components/system-theme-provider";
import "./main.css";

createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <SystemThemeProvider>
      <App />
    </SystemThemeProvider>
  </React.StrictMode>,
);
