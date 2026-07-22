import type { ReactNode } from "react";
import { Badge } from "@/components/ui/badge";
import { getBlockSplitPayload } from "@/features/sessions/session-block-split";
import { getBlockLabel } from "@/features/sessions/session-block-utils";
import { SessionBlock } from "@/features/sessions/session-block";
import type { EventBlock, SessionEvent } from "@/lib/types";
import { cn } from "@/lib/utils";

type MetaEntry = {
  title: string;
  value: ReactNode;
  destructive?: boolean;
  valueClassName?: string;
};

function readable(value: string | null | undefined) {
  return value ? value.replaceAll("_", " ") : "-";
}

function qualityBadgeVariant(value: string | null | undefined): "secondary" | "outline" | "destructive" {
  if (value === "dropped" || value === "unsupported" || value === "failed" || value === "completed_with_loss") {
    return "destructive";
  }
  return value === "preserved" || value === "exact" || value === "completed" ? "secondary" : "outline";
}

function MetaFieldRow({ title, value, destructive = false, valueClassName }: MetaEntry) {
  const content = value === null || value === undefined || value === "" ? "-" : value;

  return (
    <div className="grid grid-cols-[auto_minmax(0,1fr)] items-center gap-x-4 border-b border-border/70 py-2.5 last:border-b-0 text-sm">
      <dt className="shrink-0 text-muted-foreground">{title}</dt>
      <dd
        className={cn(
          "min-w-0 text-right break-words [overflow-wrap:anywhere]",
          destructive && "text-destructive",
          !destructive && (valueClassName ?? "text-foreground"),
        )}
      >
        {destructive ? (
          <Badge variant="destructive" className="max-w-full break-words">
            {content}
          </Badge>
        ) : (
          content
        )}
      </dd>
    </div>
  );
}

function getEventMetaEntries(event: SessionEvent): MetaEntry[] {
  return [
    {
      title: "Role",
      value: readable(event.role),
      valueClassName: "uppercase text-foreground",
    },
    {
      title: "Kind",
      value: readable(event.kind),
      valueClassName: "text-foreground",
    },
    ...(event.metadata?.model
      ? [{
          title: "Model",
          value: event.metadata.model,
          valueClassName: "font-mono text-xs",
        }]
      : []),
    ...(event.metadata?.fidelity
      ? [{
          title: "Fidelity",
          value: (
            <Badge variant={qualityBadgeVariant(event.metadata.fidelity)} className="ml-auto w-fit">
              {readable(event.metadata.fidelity)}
            </Badge>
          ),
        }]
      : []),
    ...(event.id
      ? [{
          title: "Event ID",
          value: event.id,
          valueClassName: "font-mono text-xs",
        }]
      : []),
  ];
}

function getBlockMetaEntries(block: EventBlock): MetaEntry[] {
  switch (block.type) {
    case "tool_call":
      return [
        { title: "Block", value: getBlockLabel(block), valueClassName: "text-foreground" },
        { title: "Tool", value: block.name, valueClassName: "text-foreground" },
        { title: "Tool call ID", value: block.tool_call_id, valueClassName: "font-mono text-xs" },
      ];
    case "provider_payload":
      return [
        { title: "Block", value: getBlockLabel(block), valueClassName: "text-foreground" },
        { title: "Payload kind", value: block.kind || "payload", valueClassName: "text-foreground" },
      ];
    case "command":
      return [
        { title: "Block", value: getBlockLabel(block), valueClassName: "text-foreground" },
        ...(block.cwd ? [{ title: "Working dir", value: block.cwd, valueClassName: "font-mono text-xs" }] : []),
        { title: "Command", value: block.command, valueClassName: "font-mono text-xs" },
      ];
    case "tool_result":
      return [
        { title: "Block", value: getBlockLabel(block), valueClassName: "text-foreground" },
        ...(block.is_error ? [{ title: "Status", value: "Error", destructive: true }] : []),
        { title: "Tool call ID", value: block.tool_call_id, valueClassName: "font-mono text-xs" },
      ];
    case "file":
      return [
        { title: "Block", value: getBlockLabel(block), valueClassName: "text-foreground" },
        { title: "Path", value: block.path, valueClassName: "font-mono text-xs" },
      ];
    case "patch":
      return [
        { title: "Block", value: getBlockLabel(block), valueClassName: "text-foreground" },
        ...(block.summary ? [{ title: "Summary", value: block.summary, valueClassName: "text-foreground" }] : []),
        ...(block.files?.length ? [{ title: "Files", value: block.files.join(", "), valueClassName: "font-mono text-xs" }] : []),
      ];
    case "command_result":
      return [
        { title: "Block", value: getBlockLabel(block), valueClassName: "text-foreground" },
        ...(block.command ? [{ title: "Command", value: block.command, valueClassName: "font-mono text-xs" }] : []),
        ...(block.exit_code != null ? [{ title: "Exit code", value: String(block.exit_code), valueClassName: "text-foreground" }] : []),
      ];
    case "thinking":
      return [{ title: "Block", value: getBlockLabel(block), valueClassName: "text-foreground" }];
    case "text":
      return [{ title: "Block", value: "Text", valueClassName: "text-foreground" }];
    case "image":
      return [
        { title: "Block", value: getBlockLabel(block), valueClassName: "text-foreground" },
        ...(block.path ? [{ title: "Path", value: block.path, valueClassName: "font-mono text-xs" }] : []),
        ...(block.mime_type ? [{ title: "MIME", value: block.mime_type, valueClassName: "font-mono text-xs" }] : []),
      ];
    case "compressed":
      return [
        { title: "Block", value: getBlockLabel(block), valueClassName: "text-foreground" },
        { title: "Summary", value: block.summary, valueClassName: "text-foreground" },
        ...(block.archive_ref ? [{ title: "Archive", value: block.archive_ref, valueClassName: "font-mono text-xs" }] : []),
      ];
    case "unknown":
      return [{ title: "Block", value: "Unknown", valueClassName: "text-foreground" }];
    default:
      return [{ title: "Block", value: getBlockLabel(block as EventBlock), valueClassName: "text-foreground" }];
  }
}

function SessionBlockMetaBody({ block }: { block: EventBlock }) {
  // Left column is metadata-only; message/text bodies belong in the JSON column or the full-width article.
  if (getBlockSplitPayload(block) || block.type === "text" || block.type === "thinking") {
    return null;
  }

  return (
    <div className="pt-2">
      <SessionBlock block={block} embedded />
    </div>
  );
}

export function SessionEventMetaPanel({
  event,
  eventNumber,
  className,
}: {
  event: SessionEvent;
  eventNumber: number;
  className?: string;
}) {
  const blocks = event.blocks ?? [];
  const entries = [
    ...getEventMetaEntries(event),
    ...blocks.flatMap((block) => getBlockMetaEntries(block)),
  ];

  return (
    <div className={cn("mx-auto flex w-full min-w-0 max-w-md flex-col gap-1 px-2 lg:mx-0 lg:max-w-none lg:px-3", className)} data-session-event-meta>
      <div className="border-b border-border pb-2.5">
        <h3 className="text-sm font-semibold tracking-tight">Event #{eventNumber}</h3>
      </div>

      {entries.length === 0 ? (
        <p className="py-2 text-sm text-muted-foreground">No metadata.</p>
      ) : (
        <dl className="min-w-0">
          {entries.map((entry, index) => (
            <MetaFieldRow key={`${entry.title}-${index}`} {...entry} />
          ))}
        </dl>
      )}

      {blocks.map((block, blockIndex) => (
        <SessionBlockMetaBody key={`${event.id}-body-${blockIndex}`} block={block} />
      ))}
    </div>
  );
}
