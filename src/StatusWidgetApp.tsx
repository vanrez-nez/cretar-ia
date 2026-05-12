import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { cursorPosition } from "@tauri-apps/api/window";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { useCallback, useEffect, useRef, useState } from "react";

type StatusWidgetState = "idle" | "cancelled" | "recording" | "transcribing" | "recovering" | "error";

type StatusWidgetPayload = {
  label: string;
  state: StatusWidgetState;
  expanded: boolean;
  source: string;
  session_id: number;
  phase_elapsed_ms: number;
  mic_active: boolean;
  error: string | null;
};

type StatusWidgetAudioLevelPayload = {
  levels: number[];
};

const DEFAULT_STATUS: StatusWidgetPayload = {
  label: "Idle",
  state: "idle",
  expanded: false,
  source: "",
  session_id: 0,
  phase_elapsed_ms: 0,
  mic_active: false,
  error: null,
};

const CURSOR_POLL_INTERVAL_MS = 50;
const WAVEFORM_BAR_COUNT = 12;
const WAVEFORM_BAR_WIDTH_PX = 2;
const WAVEFORM_BAR_GAP_PX = 1;
const WAVEFORM_BAR_HEIGHT_PX = 36;
const WAVEFORM_BAR_MIN_HEIGHT_PX = 2;
const WAVEFORM_DECAY_PER_SECOND = 1.0;
const WAVEFORM_MIN_ALPHA = 0.25;
const WAVEFORM_LEVEL_FLOOR = 0.035;
const WAVEFORM_SMOOTHING_FACTOR = 0.5;
const WAVEFORM_WIDTH_PX =
  WAVEFORM_BAR_COUNT * WAVEFORM_BAR_WIDTH_PX +
  (WAVEFORM_BAR_COUNT - 1) * WAVEFORM_BAR_GAP_PX;

function emptyLevels() {
  return new Array<number>(WAVEFORM_BAR_COUNT).fill(0);
}

function clampLevel(level: number) {
  if (!Number.isFinite(level)) {
    return 0;
  }
  const clamped = Math.max(0, Math.min(1, level));
  return clamped < WAVEFORM_LEVEL_FLOOR ? 0 : clamped;
}

function spectrumLevels(levels: number[]) {
  if (!Array.isArray(levels) || levels.length === 0) {
    return emptyLevels();
  }

  return emptyLevels().map((_, index) => clampLevel(levels[index] ?? 0));
}

function smoothedLevels(previous: number[], next: number[]) {
  return emptyLevels().map((_, index) => {
    const prior = previous[index] ?? 0;
    const incoming = next[index] ?? 0;
    return prior + (incoming - prior) * WAVEFORM_SMOOTHING_FACTOR;
  });
}

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

function LiveRecordingWaveform({ active }: { active: boolean }) {
  const barRefs = useRef<Array<HTMLSpanElement | null>>([]);
  const levelsRef = useRef<number[]>(emptyLevels());
  const activeRef = useRef(active);
  const decayFrameRef = useRef<number | null>(null);
  const lastDecayAtRef = useRef<number | null>(null);

  const renderBars = useCallback(() => {
    levelsRef.current.forEach((level, index) => {
      const bar = barRefs.current[index];
      if (!bar) {
        return;
      }

      const height = Math.max(WAVEFORM_BAR_MIN_HEIGHT_PX, WAVEFORM_BAR_HEIGHT_PX * level);
      const alpha = WAVEFORM_MIN_ALPHA + (1 - WAVEFORM_MIN_ALPHA) * level;
      bar.style.height = `${Math.round(height)}px`;
      bar.style.opacity = alpha.toFixed(3);
    });
  }, []);

  const animateDecay = useCallback(
    (timestamp: number) => {
      if (!activeRef.current) {
        decayFrameRef.current = null;
        lastDecayAtRef.current = null;
        return;
      }

      const previousTimestamp = lastDecayAtRef.current ?? timestamp;
      const elapsedSeconds = Math.min((timestamp - previousTimestamp) / 1000, 0.05);
      lastDecayAtRef.current = timestamp;

      let changed = false;
      levelsRef.current = levelsRef.current.map((level) => {
        let next = Math.max(0, level - WAVEFORM_DECAY_PER_SECOND * elapsedSeconds);
        if (next < WAVEFORM_LEVEL_FLOOR) {
          next = 0;
        }
        if (next !== level) {
          changed = true;
        }
        return next;
      });

      if (changed) {
        renderBars();
      }

      decayFrameRef.current = window.requestAnimationFrame(animateDecay);
    },
    [renderBars],
  );

  const startDecayAnimation = useCallback(() => {
    if (decayFrameRef.current !== null) {
      return;
    }

    lastDecayAtRef.current = null;
    decayFrameRef.current = window.requestAnimationFrame(animateDecay);
  }, [animateDecay]);

  const resetLevels = useCallback(() => {
    levelsRef.current = emptyLevels();
    renderBars();
  }, [renderBars]);

  useEffect(() => {
    activeRef.current = active;
    if (!active) {
      resetLevels();
      if (decayFrameRef.current !== null) {
        window.cancelAnimationFrame(decayFrameRef.current);
        decayFrameRef.current = null;
      }
      lastDecayAtRef.current = null;
    }
  }, [active, resetLevels]);

  useEffect(() => {
    if (!active) {
      return;
    }

    renderBars();
    startDecayAnimation();

    return () => {
      if (decayFrameRef.current !== null) {
        window.cancelAnimationFrame(decayFrameRef.current);
        decayFrameRef.current = null;
      }
      lastDecayAtRef.current = null;
    };
  }, [active, renderBars, startDecayAnimation]);

  useEffect(() => {
    let listening = true;
    const levelUnlisten = listen<StatusWidgetAudioLevelPayload>(
      "status-widget:audio-level",
      (event) => {
        if (!listening || !activeRef.current) {
          return;
        }

        const incoming = smoothedLevels(levelsRef.current, spectrumLevels(event.payload.levels));
        let changed = false;
        levelsRef.current = levelsRef.current.map((level, index) => {
          const next = Math.max(level, incoming[index] ?? 0);
          if (next !== level) {
            changed = true;
          }
          return next;
        });
        if (changed) {
          renderBars();
        }
      },
    );
    const resetUnlisten = listen("status-widget:audio-level-reset", () => {
      resetLevels();
    });

    return () => {
      listening = false;
      void levelUnlisten.then((dispose) => dispose());
      void resetUnlisten.then((dispose) => dispose());
    };
  }, [renderBars, resetLevels]);

  return (
    <span
      aria-hidden="true"
      className={`status-widget__waveform ${
        active ? "status-widget__waveform--active" : "status-widget__waveform--inactive"
      }`}
      style={{
        width: WAVEFORM_WIDTH_PX,
        height: WAVEFORM_BAR_HEIGHT_PX,
        columnGap: WAVEFORM_BAR_GAP_PX,
      }}
    >
      {Array.from({ length: WAVEFORM_BAR_COUNT }, (_, index) => (
        <span
          key={index}
          ref={(element) => {
            barRefs.current[index] = element;
          }}
          className="status-widget__waveform-bar"
          style={{
            width: WAVEFORM_BAR_WIDTH_PX,
          }}
        />
      ))}
    </span>
  );
}

export default function StatusWidgetApp() {
  const [status, setStatus] = useState<StatusWidgetPayload>(DEFAULT_STATUS);
  const [hovered, setHovered] = useState(false);
  const widgetRef = useRef<HTMLDivElement>(null);
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
  const showWaveform = status.mic_active;
  const showLoader =
    expanded &&
    !showWaveform &&
    status.state !== "idle" &&
    status.state !== "cancelled" &&
    status.state !== "error";
  const showDot = expanded && !showWaveform && !showLoader;
  const visualStateClass = showWaveform
    ? "status-widget--waveform"
    : expanded
      ? "status-widget--circle"
      : "status-widget--bar";

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
          const inside = isInsideRoundedRect(
            localX - rect.left,
            localY - rect.top,
            rect.width,
            rect.height,
          );

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
      aria-live="polite"
      aria-label={status.label}
      className="status-widget-shell"
    >
      <div
        ref={widgetRef}
        className={`status-widget status-widget--${status.state} ${visualStateClass} ${
          expanded ? "status-widget--expanded" : "status-widget--collapsed"
        }`}
        onMouseEnter={() => setHoverState(true)}
        onMouseLeave={() => setHoverState(false)}
      >
        {showWaveform && <LiveRecordingWaveform active={showWaveform} />}
        {showLoader && <span aria-hidden="true" className="status-widget__loader" />}
        {showDot && <span aria-hidden="true" className="status-widget__dot" />}
      </div>
    </main>
  );
}
