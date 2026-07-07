import { Link } from "react-router-dom";
import { Badge } from "@/components/ui/badge";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Separator } from "@/components/ui/separator";
import { renderHighlightedJson } from "@/lib/format-content";
import { cn } from "@/lib/utils";
import type { EventBlock } from "@/lib/types";

function formatJson(value: unknown) {
  if (value === undefined || value === null) return "";
  if (typeof value === "string") return value;
  return JSON.stringify(value, null, 2);
}

function CodeBlock({ value, variant = "default" }: { value: unknown; variant?: "default" | "tool" }) {
  const text = formatJson(value);
  if (!text) return <span className="text-muted-foreground">-</span>;

  const highlighted = renderHighlightedJson(value);

  return (
    <ScrollArea
      className={cn(
        "max-h-80 rounded-md border border-border bg-muted/40",
        variant === "tool" && "bg-sky-50 dark:bg-sky-950/30",
      )}
    >
      {highlighted ? (
        <pre
          className="json-block whitespace-pre-wrap break-words p-3 font-mono text-xs"
          dangerouslySetInnerHTML={{ __html: `<code>${highlighted}</code>` }}
        />
      ) : (
        <pre className="whitespace-pre-wrap break-words p-3 font-mono text-xs">{text}</pre>
      )}
    </ScrollArea>
  );
}

function FileList({ files }: { files: string[] | undefined }) {
  if (!files || files.length === 0) return null;
  return (
    <div className="flex flex-wrap gap-2">
      {files.map((file) => (
        <Badge key={file} variant="outline">
          {file}
        </Badge>
      ))}
    </div>
  );
}

function imageSource(block: Extract<EventBlock, { type: "image" }>) {
  if (!block.data) return null;
  if (block.data.startsWith("data:")) return block.data;
  return `data:${block.mime_type};base64,${block.data}`;
}

function compressionArchiveHref(archiveRef: string) {
  return `/compression?archive_ref=${encodeURIComponent(archiveRef)}`;
}

export function SessionBlock({ block }: { block: EventBlock }) {
  switch (block.type) {
    case "text":
      return <CodeBlock value={block.text} />;
    case "thinking":
      return <CodeBlock value={block.text} />;
    case "tool_call":
      return <CodeBlock variant="tool" value={{ tool_call_id: block.tool_call_id, name: block.name, input: block.input }} />;
    case "tool_result":
      return (
        <div className="flex flex-col gap-2">
          {block.is_error ? <Badge variant="destructive">Error</Badge> : null}
          <CodeBlock variant="tool" value={block.content} />
        </div>
      );
    case "patch":
      return (
        <div className="flex flex-col gap-3">
          {block.summary ? <p className="text-sm">{block.summary}</p> : null}
          <FileList files={block.files} />
          {block.diff_text ? <CodeBlock value={block.diff_text} /> : null}
        </div>
      );
    case "command":
      return (
        <div className="flex flex-col gap-3">
          {block.cwd ? <p className="font-mono text-xs text-muted-foreground">{block.cwd}</p> : null}
          <CodeBlock value={{ command: block.command, argv: block.argv, cwd: block.cwd }} />
        </div>
      );
    case "command_result":
      return (
        <div className="flex flex-col gap-3">
          {block.command ? <CodeBlock value={block.command} /> : null}
          {block.stdout ? <CodeBlock value={block.stdout} /> : null}
          {block.stderr ? (
            <>
              <Separator />
              <CodeBlock value={block.stderr} />
            </>
          ) : null}
        </div>
      );
    case "file":
      return (
        <div className="flex flex-col gap-2">
          <code className="break-all font-mono text-xs">{block.path}</code>
          <CodeBlock value={block.content} />
        </div>
      );
    case "image": {
      const src = imageSource(block);
      return src ? (
        <div className="flex flex-col gap-2">
          {block.path || block.mime_type ? (
            <code className="break-all font-mono text-xs text-muted-foreground">{block.path ?? block.mime_type}</code>
          ) : null}
          <img alt={block.path ?? "Session image"} className="max-h-96 rounded-md object-contain" src={src} />
        </div>
      ) : (
        <CodeBlock value={block} />
      );
    }
    case "provider_payload":
      return <CodeBlock value={block.payload} />;
    case "compressed": {
      const archiveRef = block.archive_ref || "";
      return (
        <div className="flex flex-col gap-3">
          {archiveRef ? (
            <Link
              to={compressionArchiveHref(archiveRef)}
              className="rounded-md border bg-muted/30 p-3 transition-colors hover:bg-muted"
              data-compression-detail-link
            >
              <p className="text-sm font-medium">{block.summary}</p>
              <p className="mt-2 break-all font-mono text-xs text-muted-foreground">{archiveRef}</p>
            </Link>
          ) : (
            <p className="text-sm">{block.summary}</p>
          )}
          {block.source_event_count === null || block.source_event_count === undefined ? null : (
            <Badge variant="secondary">{block.source_event_count} source events</Badge>
          )}
          <FileList files={block.source_event_ids} />
        </div>
      );
    }
    case "unknown":
      return <CodeBlock value={block.raw} />;
    default: {
      const unknownBlock = block as { type?: string } & Record<string, unknown>;
      return <CodeBlock value={unknownBlock} />;
    }
  }
}
