import type { ReactNode } from "react";

export function SettingsItemEditor({
  isLocked,
  children,
}: {
  isLocked?: boolean;
  children: ReactNode;
}) {
  return (
    <div className={`space-y-3 rounded-lg bg-muted/35 p-3 ${isLocked ? "opacity-60" : ""}`}>
      {children}
    </div>
  );
}
