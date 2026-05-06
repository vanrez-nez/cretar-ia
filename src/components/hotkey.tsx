import { useEffect, useRef, useState } from "react";
import type { KeyboardEvent as ReactKeyboardEvent } from "react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { ArrowBigUp, Command, CornerDownLeft, Option } from "lucide-react";

const MODIFIER_ORDER = ["ctrl", "alt", "shift", "cmd"] as const;

type ModifierToken = (typeof MODIFIER_ORDER)[number];
type ShortcutPart = ModifierToken | string;

type HotkeyCaptureProps = {
  shortcut: string;
  disabled?: boolean;
  onChange: (shortcut: string) => void;
};

export function HotkeyCapture({ shortcut, disabled, onChange }: HotkeyCaptureProps) {
  const [isEditing, setIsEditing] = useState(false);
  const [capturedModifiers, setCapturedModifiers] = useState<ModifierToken[]>([]);
  const captureRef = useRef<HTMLDivElement | null>(null);
  const displayParts = isEditing ? capturedModifiers : parseShortcutParts(shortcut);

  useEffect(() => {
    if (!isEditing) {
      return;
    }
    setCapturedModifiers([]);
    window.setTimeout(() => captureRef.current?.focus(), 0);
  }, [isEditing]);

  const cancel = () => {
    setIsEditing(false);
    setCapturedModifiers([]);
  };

  const handleKeyDown = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    if (!isEditing || event.repeat) {
      return;
    }

    event.preventDefault();
    event.stopPropagation();

    if (event.key === "Escape") {
      cancel();
      return;
    }

    const modifier = modifierFromEvent(event);
    if (modifier) {
      setCapturedModifiers((current) => orderedUniqueModifiers([...current, modifier]));
      return;
    }

    const trigger = triggerFromEvent(event);
    if (!trigger) {
      return;
    }

    const modifiers = orderedUniqueModifiers([
      ...capturedModifiers,
      ...modifiersFromKeyboardEvent(event),
    ]);
    onChange([...modifiers, trigger].join("+"));
    setIsEditing(false);
    setCapturedModifiers([]);
  };

  return (
    <div className="flex flex-wrap items-center justify-end gap-2">
      <div
        ref={captureRef}
        tabIndex={isEditing ? 0 : -1}
        onKeyDown={handleKeyDown}
        className="flex min-h-8 flex-wrap items-center gap-1 outline-none"
        aria-label="Hotkey capture"
      >
        {isEditing && displayParts.length === 0 ? (
          <>
            <span className="mr-1 text-xs text-muted-foreground">e.g.</span>
            <HotkeyBadge part="shift" tone="example" />
            <HotkeyBadge part="cmd" tone="example" />
            <HotkeyBadge part="space" tone="example" />
          </>
        ) : null}
        {displayParts.map((part, index) => (
          <HotkeyBadge key={`${part}-${index}`} part={part} tone={isEditing ? "capture" : "defined"} />
        ))}
        {isEditing && displayParts.length > 0 ? <HotkeyBadge part="?" pending /> : null}
        {!isEditing && displayParts.length === 0 ? <HotkeyBadge part="?" pending /> : null}
      </div>
      <Button
        type="button"
        variant={isEditing ? "secondary" : "outline"}
        size="sm"
        disabled={disabled}
        onClick={() => {
          if (isEditing) {
            cancel();
          } else {
            setIsEditing(true);
          }
        }}
      >
        {isEditing ? "Cancel" : "Edit"}
      </Button>
    </div>
  );
}

function HotkeyBadge({
  part,
  pending = false,
  tone = "defined",
}: {
  part: ShortcutPart | "?";
  pending?: boolean;
  tone?: "defined" | "capture" | "example";
}) {
  const icon = iconForShortcutPart(part);
  return (
    <Badge
      variant={pending ? "outline" : "secondary"}
      className={cn(
        "h-7 gap-1.5 rounded-md px-2 font-mono text-[11px] uppercase tracking-wide",
        tone === "defined" && "bg-muted/40 text-white",
        tone === "capture" && "border-emerald-400/70 bg-emerald-500/20 text-emerald-100",
        tone === "example" && "border-border/50 bg-muted/30 text-muted-foreground opacity-70"
      )}
    >
      {icon}
      {labelForShortcutPart(part)}
    </Badge>
  );
}

function parseShortcutParts(shortcut: string): ShortcutPart[] {
  return shortcut
    .split("+")
    .map((part) => normalizeShortcutToken(part))
    .filter(Boolean);
}

function normalizeShortcutToken(token: string): string {
  const value = token.trim().toLowerCase();
  if (value === "control") {
    return "ctrl";
  }
  if (value === "option") {
    return "alt";
  }
  if (value === "command" || value === "meta" || value === "win") {
    return "cmd";
  }
  return value;
}

function modifierFromEvent(event: ReactKeyboardEvent<HTMLDivElement>): ModifierToken | null {
  switch (event.key) {
    case "Control":
      return "ctrl";
    case "Alt":
      return "alt";
    case "Shift":
      return "shift";
    case "Meta":
      return "cmd";
    default:
      return null;
  }
}

function modifiersFromKeyboardEvent(event: ReactKeyboardEvent<HTMLDivElement>): ModifierToken[] {
  const modifiers: ModifierToken[] = [];
  if (event.ctrlKey) {
    modifiers.push("ctrl");
  }
  if (event.altKey) {
    modifiers.push("alt");
  }
  if (event.shiftKey) {
    modifiers.push("shift");
  }
  if (event.metaKey) {
    modifiers.push("cmd");
  }
  return orderedUniqueModifiers(modifiers);
}

function orderedUniqueModifiers(values: ModifierToken[]): ModifierToken[] {
  const set = new Set(values);
  return MODIFIER_ORDER.filter((modifier) => set.has(modifier));
}

function triggerFromEvent(event: ReactKeyboardEvent<HTMLDivElement>): string | null {
  if (/^Key[A-Z]$/.test(event.code)) {
    return event.code.slice(3).toLowerCase();
  }
  if (/^Digit[0-9]$/.test(event.code)) {
    return event.code.slice(5);
  }
  switch (event.code) {
    case "Space":
      return "space";
    case "Enter":
    case "NumpadEnter":
      return "enter";
    case "Tab":
      return "tab";
    case "CapsLock":
      return "caps";
    default:
      return null;
  }
}

function labelForShortcutPart(part: ShortcutPart | "?"): string {
  switch (part) {
    case "ctrl":
      return "Ctrl";
    case "alt":
      return "Option";
    case "shift":
      return "Shift";
    case "cmd":
      return "Command";
    case "space":
      return "SPACE";
    case "enter":
      return "ENTER";
    case "tab":
      return "TAB";
    case "caps":
      return "CAPS";
    case "?":
      return "?";
    default:
      return part.toUpperCase();
  }
}

function iconForShortcutPart(part: ShortcutPart | "?") {
  const className = "size-3";
  switch (part) {
    case "alt":
      return <Option className={className} aria-hidden="true" />;
    case "shift":
      return <ArrowBigUp className={className} aria-hidden="true" />;
    case "cmd":
      return <Command className={className} aria-hidden="true" />;
    case "ctrl":
      return <CornerDownLeft className={className} aria-hidden="true" />;
    default:
      return null;
  }
}
