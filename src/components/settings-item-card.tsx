import type { KeyboardEvent, ReactNode } from "react";

type SettingsItemCardProps = {
  isLocked: boolean;
  isSelected: boolean;
  onSelect?: () => void;
  children: ReactNode;
  actions?: ReactNode;
  status?: ReactNode;
};

export function SettingsItemCard({
  isLocked,
  isSelected,
  onSelect,
  children,
  actions,
  status,
}: SettingsItemCardProps) {
  const handleKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (!onSelect || isLocked) {
      return;
    }
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      onSelect();
    }
  };

  return (
    <div
      className={`group flex cursor-pointer items-center justify-between gap-3 rounded-lg border p-3 transition-colors ${
        isLocked
          ? "cursor-default border-transparent bg-muted/40 opacity-60 hover:border-transparent"
          : isSelected
          ? "border-border bg-muted/40 hover:border-border/50"
          : "border-transparent bg-muted/60 hover:border-border/50"
      }`}
      aria-disabled={isLocked || undefined}
      role={isLocked ? undefined : "button"}
      tabIndex={isLocked ? -1 : 0}
      onClick={isLocked || !onSelect ? undefined : onSelect}
      onKeyDown={handleKeyDown}
    >
      <div className="min-w-0 select-none">{children}</div>
      <div className="flex shrink-0 items-center gap-2">
        {!isLocked && actions ? (
          <div className="flex items-center gap-2 opacity-0 transition-opacity group-hover:opacity-100 group-focus-within:opacity-100">
            {actions}
          </div>
        ) : null}
        {status}
      </div>
    </div>
  );
}
