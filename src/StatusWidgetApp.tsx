import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { cursorPosition } from "@tauri-apps/api/window";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { useCallback, useEffect, useRef, useState } from "react";

type StatusWidgetState = "idle" | "recording" | "transcribing" | "recovering" | "error";

type StatusWidgetPayload = {
  label: string;
  state: StatusWidgetState;
  expanded: boolean;
  source: string;
  session_id: number;
  phase_elapsed_ms: number;
  error: string | null;
};

const DEFAULT_STATUS: StatusWidgetPayload = {
  label: "Idle",
  state: "idle",
  expanded: false,
  source: "",
  session_id: 0,
  phase_elapsed_ms: 0,
  error: null,
};

const CURSOR_POLL_INTERVAL_MS = 50;

function isInsideRoundedRect(x: number, y: number, width: number, height: number) {
  if (x < 0 || y < 0 || x > width || y > height) {
    return false;
  }

  const radius = Math.min(width, height) / 2;
  if ((x >= radius && x <= width - radius) || (y >= radius && y <= height - radius)) {
    return true;
  }

  const cornerX = x < radius ? radius : width - radius;
  const cornerY = y < radius ? radius : height - radius;
  return Math.hypot(x - cornerX, y - cornerY) <= radius;
}

export default function StatusWidgetApp() {
  const [status, setStatus] = useState<StatusWidgetPayload>(DEFAULT_STATUS);
  const [hovered, setHovered] = useState(false);
  const widgetRef = useRef<HTMLElement>(null);
  const hoveredRef = useRef(false);
  const ignoreCursorEventsRef = useRef<boolean | null>(null);
  const ignoreCursorEventsWarnedRef = useRef(false);

  useEffect(() => {
    let active = true;
    const unlisten = listen<StatusWidgetPayload>("status-widget:update", (event) => {
      if (active) {
        setStatus(event.payload);
      }
    });

    return () => {
      active = false;
      void unlisten.then((dispose) => dispose());
    };
  }, []);

  const expanded = hovered || status.expanded;

  const setHoverState = useCallback((nextHovered: boolean) => {
    if (hoveredRef.current === nextHovered) {
      return;
    }

    hoveredRef.current = nextHovered;
    setHovered(nextHovered);
    void invoke("set_status_widget_hovered", { hovered: nextHovered }).catch((err) => {
      console.warn("failed to update status widget hover state", err);
    });
  }, []);

  useEffect(() => {
    const appWebview = getCurrentWebviewWindow();
    let cancelled = false;
    let timeoutId: number | undefined;

    const setIgnoreCursorEvents = async (ignore: boolean) => {
      if (ignoreCursorEventsRef.current === ignore) {
        return;
      }

      ignoreCursorEventsRef.current = ignore;
      try {
        await appWebview.setIgnoreCursorEvents(ignore);
      } catch (err) {
        if (!ignoreCursorEventsWarnedRef.current) {
          ignoreCursorEventsWarnedRef.current = true;
          console.warn("failed to update status widget cursor pass-through", err);
        }
      }
    };

    const pollCursor = async () => {
      if (cancelled) {
        return;
      }

      try {
        const rect = widgetRef.current?.getBoundingClientRect();
        if (rect) {
          const [cursor, position, scaleFactor] = await Promise.all([
            cursorPosition(),
            appWebview.outerPosition(),
            appWebview.scaleFactor(),
          ]);
          const localX = (cursor.x - position.x) / scaleFactor;
          const localY = (cursor.y - position.y) / scaleFactor;
          const inside = isInsideRoundedRect(localX, localY, rect.width, rect.height);

          setHoverState(inside);
          await setIgnoreCursorEvents(!inside);
        }
      } catch (err) {
        if (!ignoreCursorEventsWarnedRef.current) {
          ignoreCursorEventsWarnedRef.current = true;
          console.warn("failed to poll status widget cursor position", err);
        }
      } finally {
        if (!cancelled) {
          timeoutId = window.setTimeout(pollCursor, CURSOR_POLL_INTERVAL_MS);
        }
      }
    };

    void pollCursor();

    return () => {
      cancelled = true;
      if (timeoutId !== undefined) {
        window.clearTimeout(timeoutId);
      }
      void setIgnoreCursorEvents(false);
    };
  }, [setHoverState]);

  return (
    <main
      ref={widgetRef}
      aria-live="polite"
      className={`status-widget status-widget--${status.state} ${
        expanded ? "status-widget--expanded" : "status-widget--collapsed"
      }`}
      onMouseEnter={() => setHoverState(true)}
      onMouseLeave={() => setHoverState(false)}
    >
      <span className="status-widget__indicator" aria-hidden="true" />
      <span className="status-widget__label">{status.label}</span>
    </main>
  );
}
