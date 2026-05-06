import type { ReactNode } from "react";
import { Badge } from "@/components/ui/badge";
import { cn } from "@/lib/utils";

type SettingsBadgeTone = "default" | "success" | "danger" | "capture" | "example" | "pending";

type SettingsBadgeProps = {
  children: ReactNode;
  icon?: ReactNode;
  tone?: SettingsBadgeTone;
  className?: string;
};

export function SettingsBadge({
  children,
  icon,
  tone = "default",
  className,
}: SettingsBadgeProps) {
  return (
    <Badge
      variant={tone === "pending" ? "outline" : "secondary"}
      className={cn(
        "h-7 gap-1.5 rounded-md px-2 font-mono text-[11px] uppercase tracking-wide",
        tone === "default" && "bg-muted/40 text-white",
        tone === "success" && "border-emerald-400/70 bg-emerald-500/20 text-emerald-100",
        tone === "danger" && "border-red-400/70 bg-red-500/20 text-red-100",
        tone === "capture" && "border-emerald-400/70 bg-emerald-500/20 text-emerald-100",
        tone === "example" && "border-border/50 bg-muted/30 text-muted-foreground opacity-70",
        className
      )}
    >
      {icon}
      {children}
    </Badge>
  );
}
