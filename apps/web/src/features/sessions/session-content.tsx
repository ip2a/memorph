import type { ReactNode } from "react";
import { MarkdownContent } from "@/components/shared/markdown-content";
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

function JsonBlock({ value }: { value: unknown }) {
  const text = formatJson(value);
  if (!text) return <span className="text-muted-foreground">-</span>;

  const highlighted = renderHighlightedJson(value);
  if (highlighted) {
    return (
      <pre
        className="json-block m-0 whitespace-pre-wrap break-words font-mono text-xs leading-relaxed [overflow-wrap:anywhere]"
        dangerouslySetInnerHTML={{ __html: `<code>${highlighted}</code>` }}
      />
    );
  }

  return (
    <pre className="m-0 whitespace-pre-wrap break-words font-mono text-xs leading-relaxed [overflow-wrap:anywhere]">
      {text}
    </pre>
  );
}

function MarkdownBlock({ value }: { value: string }) {
  return <MarkdownContent value={value} />;
}

function PlainTextBlock({
  value,
  mono = false,
}: {
  value: string;
  mono?: boolean;
}) {
  return (
    <div
      className={cn(
        "text-sm leading-7 whitespace-pre-wrap break-words [overflow-wrap:anywhere]",
        mono && "font-mono text-xs leading-relaxed",
      )}
    >
      {value}
    </div>
  );
}

function LogOutputBlock({ value }: { value: string }) {
  const rows = parseLogOutput(value);

  return (
    <div className="flex flex-col gap-1">
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
    return <JsonBlock value={value} />;
  }

  if (variant === "tool") {
    if (looksLikeJson(value)) {
      return <JsonBlock value={value} />;
    }
    if (detectSessionContentKind(value) === "log") {
      return <LogOutputBlock value={value} />;
    }
    return <PlainTextBlock mono value={value} />;
  }

  switch (detectSessionContentKind(value)) {
    case "json":
      return <JsonBlock value={value} />;
    case "log":
      return <LogOutputBlock value={value} />;
    case "markdown":
      return <MarkdownBlock value={value} />;
    default:
      return <PlainTextBlock value={value} />;
  }
}

export function SessionCodeBlock({
  value,
}: {
  value: unknown;
  embedded?: boolean;
}): ReactNode {
  return <JsonBlock value={value} />;
}
