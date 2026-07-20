import type { ReactNode } from "react";
import { MarkdownContentFrame } from "@/components/shared/markdown-content";
import {
  detectSessionContentKind,
  looksLikeJson,
  parseLogOutput,
  renderHighlightedJson,
} from "@/lib/format-content";
import { cn } from "@/lib/utils";

function formatJson(value: unknown) {
  if (value === undefined || value === null) return "";
  if (typeof value === "string") return value;
  return JSON.stringify(value, null, 2);
}

function ContentFrame({
  embedded,
  className,
  children,
}: {
  embedded?: boolean;
  className?: string;
  children: ReactNode;
}) {
  if (embedded) {
    return <div className={className}>{children}</div>;
  }

  return (
    <div className={cn("max-h-80 overflow-auto rounded-md border border-border bg-muted/30", className)}>
      {children}
    </div>
  );
}

function JsonBlock({
  value,
  embedded = false,
}: {
  value: unknown;
  embedded?: boolean;
}) {
  const text = formatJson(value);
  if (!text) return <span className="text-muted-foreground">-</span>;

  const highlighted = renderHighlightedJson(value);
  const body = highlighted ? (
    <pre
      className={cn(
        "json-block whitespace-pre-wrap break-words font-mono text-xs leading-relaxed [overflow-wrap:anywhere]",
        !embedded && "p-3",
      )}
      dangerouslySetInnerHTML={{ __html: `<code>${highlighted}</code>` }}
    />
  ) : (
    <pre
      className={cn(
        "whitespace-pre-wrap break-words font-mono text-xs leading-relaxed [overflow-wrap:anywhere]",
        !embedded && "p-3",
      )}
    >
      {text}
    </pre>
  );

  return <ContentFrame embedded={embedded}>{body}</ContentFrame>;
}

function MarkdownBlock({ value, embedded = false }: { value: string; embedded?: boolean }) {
  return <MarkdownContentFrame embedded={embedded} value={value} />;
}

function PlainTextBlock({
  value,
  embedded = false,
  mono = false,
}: {
  value: string;
  embedded?: boolean;
  mono?: boolean;
}) {
  return (
    <div
      className={cn(
        "text-sm leading-7 whitespace-pre-wrap break-words [overflow-wrap:anywhere]",
        mono && "font-mono text-xs leading-relaxed",
        !embedded && mono && "rounded-md border border-border bg-muted/30 p-3",
      )}
    >
      {value}
    </div>
  );
}

function LogOutputBlock({ value, embedded = false }: { value: string; embedded?: boolean }) {
  const rows = parseLogOutput(value);

  return (
    <div className={cn("flex flex-col gap-1", !embedded && "rounded-md border border-border bg-muted/30 p-3")}>
      {rows.map((row, index) => {
        if (row.kind === "path") {
          return (
            <div key={`${index}-${row.text}`} className="break-all font-mono text-xs text-muted-foreground">
              {row.text}
            </div>
          );
        }

        if (row.kind === "match") {
          return (
            <div key={`${index}-${row.text}`} className="font-mono text-xs leading-relaxed break-words [overflow-wrap:anywhere]">
              {row.text}
            </div>
          );
        }

        return (
          <div key={`${index}-${row.text}`} className="text-sm leading-relaxed break-words [overflow-wrap:anywhere]">
            {row.text}
          </div>
        );
      })}
    </div>
  );
}

export function SessionContent({
  value,
  embedded = false,
  variant = "default",
}: {
  value: unknown;
  embedded?: boolean;
  variant?: "default" | "tool";
}) {
  if (value === undefined || value === null || value === "") {
    return <span className="text-muted-foreground">-</span>;
  }

  if (typeof value !== "string") {
    return <JsonBlock embedded={embedded} value={value} />;
  }

  if (variant === "tool") {
    if (looksLikeJson(value)) {
      return <JsonBlock embedded={embedded} value={value} />;
    }
    if (detectSessionContentKind(value) === "log") {
      return <LogOutputBlock embedded={embedded} value={value} />;
    }
    return <PlainTextBlock embedded={embedded} mono value={value} />;
  }

  switch (detectSessionContentKind(value)) {
    case "json":
      return <JsonBlock embedded={embedded} value={value} />;
    case "log":
      return <LogOutputBlock embedded={embedded} value={value} />;
    case "markdown":
      return <MarkdownBlock embedded={embedded} value={value} />;
    default:
      return <PlainTextBlock embedded={embedded} value={value} />;
  }
}

export function SessionCodeBlock({
  value,
  embedded = false,
}: {
  value: unknown;
  embedded?: boolean;
}): ReactNode {
  return <JsonBlock embedded={embedded} value={value} />;
}
