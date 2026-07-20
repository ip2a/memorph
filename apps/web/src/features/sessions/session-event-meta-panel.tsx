import type { ReactNode } from "react";
import { Badge } from "@/components/ui/badge";
import { Card } from "@/components/ui/card";
import { Field, FieldContent, FieldGroup, FieldTitle } from "@/components/ui/field";
import { Separator } from "@/components/ui/separator";
import { getBlockSplitPayload } from "@/features/sessions/session-block-split";
import { getBlockLabel } from "@/features/sessions/session-block-utils";
import { SessionBlock } from "@/features/sessions/session-block";
import { formatDateTime } from "@/lib/format";
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
    <Field orientation="horizontal" className="items-center gap-3">
      <FieldContent>
        <FieldTitle>{title}</FieldTitle>
      </FieldContent>
      {destructive ? (
        <Badge variant="destructive" className="max-w-[min(100%,16rem)] shrink-0 break-words">
          {content}
        </Badge>
      ) : (
        <div
          className={cn(
            "min-w-0 max-w-[min(100%,16rem)] shrink-0 text-right text-sm break-words [overflow-wrap:anywhere]",
            valueClassName ?? "text-muted-foreground",
          )}
        >
          {content}
        </div>
      )}
    </Field>
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
  if (getBlockSplitPayload(block)) {
    return null;
  }

  return (
    <div className="px-4 py-2">
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
    <Card
      className={cn(
        "flex w-full min-w-0 flex-col gap-0 overflow-hidden rounded-xl border border-border bg-card py-0 shadow-none ring-0",
        className,
      )}
      size="sm"
      data-session-event-meta
    >
      <div className="flex shrink-0 items-center justify-between gap-3 border-b px-4 py-3">
        <h3 className="text-sm font-semibold tracking-tight">Event #{eventNumber}</h3>
        <span className="shrink-0 text-xs text-muted-foreground">{formatDateTime(event.timestamp)}</span>
      </div>

      {entries.length === 0 ? (
        <p className="px-4 py-3 text-sm text-muted-foreground">No metadata.</p>
      ) : (
        <FieldGroup className="gap-0">
          {entries.map((entry, index) => (
            <div key={`${entry.title}-${index}`} className="px-4 py-2 sm:px-4 sm:py-2.5">
              <MetaFieldRow {...entry} />
              {index < entries.length - 1 ? <Separator className="mt-2.5" /> : null}
            </div>
          ))}
        </FieldGroup>
      )}

      {blocks.map((block, blockIndex) => (
        <SessionBlockMetaBody key={`${event.id}-body-${blockIndex}`} block={block} />
      ))}
    </Card>
  );
}
