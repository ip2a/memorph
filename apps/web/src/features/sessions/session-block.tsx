import { Link } from "react-router-dom";
import { Badge } from "@/components/ui/badge";
import { SessionCodeBlock, SessionContent } from "@/features/sessions/session-content";
import { cn } from "@/lib/utils";
import type { EventBlock } from "@/lib/types";

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

export function SessionBlock({ block, embedded = false }: { block: EventBlock; embedded?: boolean }) {
  switch (block.type) {
    case "text":
      return <SessionContent embedded={embedded} value={block.text} />;
    case "thinking":
      return <SessionContent embedded={embedded} value={block.text} />;
    case "tool_call":
      return (
        <SessionCodeBlock
          embedded={embedded}
          value={{ tool_call_id: block.tool_call_id, name: block.name, input: block.input }}
        />
      );
    case "tool_result":
      return (
        <div className="flex flex-col gap-2">
          {block.is_error ? <Badge variant="destructive">Error</Badge> : null}
          <SessionContent embedded={embedded} variant="tool" value={block.content} />
        </div>
      );
    case "patch":
      return (
        <div className="flex flex-col gap-3">
          {block.summary ? <p className="text-sm">{block.summary}</p> : null}
          <FileList files={block.files} />
          {block.diff_text ? <SessionContent embedded={embedded} value={block.diff_text} /> : null}
        </div>
      );
    case "command":
      return (
        <div className="flex flex-col gap-2">
          {block.cwd ? <p className="font-mono text-xs text-muted-foreground">{block.cwd}</p> : null}
          <div className={cn("font-mono text-xs", !embedded && "rounded-md border border-border bg-muted/30 p-3")}>
            <span className="text-muted-foreground">$ </span>
            <span className="break-words [overflow-wrap:anywhere]">{block.command}</span>
          </div>
        </div>
      );
    case "command_result":
      return (
        <div className="flex flex-col gap-2">
          {block.stdout ? <SessionContent embedded={embedded} value={block.stdout} /> : null}
          {block.stderr ? (
            <div className="font-mono text-xs leading-relaxed whitespace-pre-wrap break-words text-destructive [overflow-wrap:anywhere]">
              {block.stderr}
            </div>
          ) : null}
          {!block.stdout && !block.stderr ? <span className="text-sm text-muted-foreground">(No output)</span> : null}
        </div>
      );
    case "file":
      return (
        <div className="flex flex-col gap-2">
          <code className="break-all font-mono text-xs">{block.path}</code>
          <SessionContent embedded={embedded} value={block.content} />
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
        <SessionCodeBlock embedded={embedded} value={block} />
      );
    }
    case "provider_payload":
      return <SessionCodeBlock embedded={embedded} value={block.payload} />;
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
      return <SessionContent embedded={embedded} value={block.raw} />;
    default: {
      const unknownBlock = block as { type?: string } & Record<string, unknown>;
      return <SessionCodeBlock embedded={embedded} value={unknownBlock} />;
    }
  }
}
