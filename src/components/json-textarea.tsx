import { useMemo, useRef, type TextareaHTMLAttributes } from "react";
import hljs from "highlight.js/lib/core";
import json from "highlight.js/lib/languages/json";

if (!hljs.getLanguage("json")) {
  hljs.registerLanguage("json", json);
}

type JsonTextareaProps = Omit<
  TextareaHTMLAttributes<HTMLTextAreaElement>,
  "onChange" | "value"
> & {
  value: string;
  onChange: (value: string) => void;
  containerClassName?: string;
};

export function JsonTextarea({
  value,
  onChange,
  containerClassName = "",
  ...props
}: JsonTextareaProps) {
  const highlightRef = useRef<HTMLPreElement>(null);

  const highlighted = useMemo(() => {
    const source = value || " ";
    try {
      return hljs.highlight(source, {
        language: "json",
        ignoreIllegals: true,
      }).value;
    } catch {
      return escapeHtml(source);
    }
  }, [value]);

  return (
    <div
      className={`relative min-h-28 overflow-hidden rounded-md bg-background ring-1 ring-border focus-within:ring-ring ${containerClassName}`}
    >
      <pre
        ref={highlightRef}
        aria-hidden="true"
        className="pointer-events-none absolute inset-0 m-0 min-h-28 overflow-hidden whitespace-pre-wrap break-words p-2 font-mono text-xs leading-5 text-foreground"
      >
        <code
          className="hljs language-json"
          dangerouslySetInnerHTML={{ __html: highlighted }}
        />
      </pre>
      <textarea
        {...props}
        spellCheck={props.spellCheck ?? false}
        value={value}
        onChange={(event) => onChange(event.target.value)}
        onScroll={(event) => {
          if (highlightRef.current) {
            highlightRef.current.scrollTop = event.currentTarget.scrollTop;
            highlightRef.current.scrollLeft = event.currentTarget.scrollLeft;
          }
          props.onScroll?.(event);
        }}
        className="relative z-10 min-h-28 w-full resize-y overflow-auto rounded-md bg-transparent p-2 font-mono text-xs leading-5 text-transparent caret-foreground outline-none selection:bg-primary/20"
      />
    </div>
  );
}

function escapeHtml(value: string): string {
  return value
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#39;");
}
