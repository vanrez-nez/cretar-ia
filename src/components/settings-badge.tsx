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
        tone === "default" && "bg-muted text-muted-foreground",
        tone === "success" && "border-success/40 bg-success-muted text-success",
        tone === "danger" && "border-destructive/40 bg-destructive-muted text-destructive",
        tone === "capture" && "border-primary/30 bg-primary text-primary-foreground",
        tone === "example" && "border-border/50 bg-muted/50 text-muted-foreground opacity-75",
        tone === "pending" && "border-border/60 bg-background text-muted-foreground",
        className
      )}
    >
      {icon}
      {children}
    </Badge>
  );
}
