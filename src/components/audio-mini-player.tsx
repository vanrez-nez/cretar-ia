import { convertFileSrc, isTauri } from "@tauri-apps/api/core";
import { Play, Square } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import WaveSurfer from "wavesurfer.js";

import { Button } from "@/components/ui/button";
import { Card, CardContent, CardFooter } from "@/components/ui/card";
import { logger } from "@/lib/logger";

export type AudioWaveform = {
  duration: number;
  peaks: number[][];
};

type AudioMiniPlayerProps = {
  audioFilePath: string;
  waveform: AudioWaveform;
};

export function AudioMiniPlayer({ audioFilePath, waveform }: AudioMiniPlayerProps) {
  const { t } = useTranslation();
  const containerRef = useRef<HTMLDivElement | null>(null);
  const wavesurferRef = useRef<WaveSurfer | null>(null);
  const [isReady, setIsReady] = useState(false);
  const [isPlaying, setIsPlaying] = useState(false);
  const [currentTime, setCurrentTime] = useState(0);
  const [error, setError] = useState<string | null>(null);

  const audioUrl = useMemo(() => {
    if (!isTauri()) {
      return "";
    }
    return convertFileSrc(audioFilePath);
  }, [audioFilePath]);

  useEffect(() => {
    const container = containerRef.current;
    if (!container || !audioUrl) {
      return;
    }

    setIsReady(false);
    setIsPlaying(false);
    setCurrentTime(0);
    setError(null);

    const styles = getComputedStyle(document.documentElement);
    const foreground = styles.getPropertyValue("--foreground").trim() || "#111827";
    const mutedForeground = styles.getPropertyValue("--muted-foreground").trim() || "#6b7280";

    const wavesurfer = WaveSurfer.create({
      container,
      url: audioUrl,
      peaks: waveform.peaks,
      duration: waveform.duration,
      backend: "MediaElement",
      height: 38,
      barWidth: 2,
      barGap: 1,
      barRadius: 2,
      cursorWidth: 2,
      cursorColor: foreground,
      waveColor: mutedForeground,
      progressColor: foreground,
      normalize: true,
    });

    wavesurferRef.current = wavesurfer;

    wavesurfer.on("ready", () => setIsReady(true));
    wavesurfer.on("play", () => setIsPlaying(true));
    wavesurfer.on("pause", () => setIsPlaying(false));
    wavesurfer.on("timeupdate", (time) => setCurrentTime(time));
    wavesurfer.on("seeking", (time) => setCurrentTime(time));
    wavesurfer.on("finish", () => {
      setIsPlaying(false);
      setCurrentTime(0);
      wavesurfer.seekTo(0);
    });
    wavesurfer.on("error", (error) => {
      const message = error instanceof Error ? error.message : String(error);
      logger.warn("Audio player failed", { message });
      setError(message);
      setIsReady(false);
      setIsPlaying(false);
    });

    return () => {
      wavesurferRef.current = null;
      wavesurfer.destroy();
    };
  }, [audioUrl, waveform.duration, waveform.peaks]);

  const play = () => {
    void wavesurferRef.current?.play();
  };

  const stop = () => {
    const wavesurfer = wavesurferRef.current;
    if (!wavesurfer) {
      return;
    }
    wavesurfer.pause();
    wavesurfer.seekTo(0);
    setCurrentTime(0);
    setIsPlaying(false);
  };

  return (
    <Card className="border-border/40 bg-muted/40 shadow-none">
      <CardContent>
        <div ref={containerRef} className="w-full" />
      </CardContent>
      <CardFooter className="flex min-h-8 items-center justify-between gap-2 border-t-0 py-1.5">
        <div className="text-xs tabular-nums text-muted-foreground">
          {formatDuration(currentTime)} / {formatDuration(waveform.duration)}
        </div>
        <div className="flex items-center gap-2">
          <Button
            size="icon-sm"
            variant="ghost"
            disabled={!isReady || isPlaying}
            aria-label={t("history.play")}
            title={t("history.play")}
            onClick={play}
          >
            <Play className="size-4" aria-hidden="true" />
          </Button>
          <Button
            size="icon-sm"
            variant="ghost"
            disabled={!isReady}
            aria-label={t("history.stop")}
            title={t("history.stop")}
            onClick={stop}
          >
            <Square className="size-4" aria-hidden="true" />
          </Button>
        </div>
      </CardFooter>
      {error ? <CardFooter className="pt-0"><p className="text-xs text-destructive">{error}</p></CardFooter> : null}
    </Card>
  );
}

function formatDuration(seconds: number) {
  if (!Number.isFinite(seconds) || seconds <= 0) {
    return "0:00";
  }
  const totalSeconds = Math.round(seconds);
  const minutes = Math.floor(totalSeconds / 60);
  const rest = totalSeconds % 60;
  return `${minutes}:${String(rest).padStart(2, "0")}`;
}
