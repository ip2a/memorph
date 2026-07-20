import { Badge } from "@/components/ui/badge";
import { getBlockLabel } from "@/features/sessions/session-block-utils";
import { looksLikeJson } from "@/lib/format-content";
import type { EventBlock } from "@/lib/types";
import type { ReactNode } from "react";
import { SessionBlock } from "@/features/sessions/session-block";

export type BlockSplitPayload = {
  json: unknown;
  jsonLabel: string;
};

export function getBlockSplitPayload(block: EventBlock): BlockSplitPayload | null {
  switch (block.type) {
    case "text":
    case "thinking":
    case "patch":
    case "command_result":
    case "image":
    case "compressed":
      return null;
    case "tool_call":
      return {
        json: { tool_call_id: block.tool_call_id, name: block.name, input: block.input },
        jsonLabel: "Request",
      };
    case "provider_payload":
      return {
        json: block.payload,
        jsonLabel: block.kind || "Payload",
      };
    case "command":
      return {
        json: { command: block.command, argv: block.argv, cwd: block.cwd },
        jsonLabel: "Command",
      };
    case "tool_result": {
      const content = block.content;
      if (typeof content === "string" && looksLikeJson(content)) {
        try {
          return { json: JSON.parse(content), jsonLabel: "Response" };
        } catch {
          return null;
        }
      }
      return null;
    }
    case "file": {
      const content = block.content;
      if (content && looksLikeJson(content)) {
        try {
          return { json: JSON.parse(content), jsonLabel: "Content" };
        } catch {
          return null;
        }
      }
      return null;
    }
    case "unknown": {
      if (block.raw == null) return null;
      if (typeof block.raw === "object") {
        return { json: block.raw, jsonLabel: "Raw" };
      }
      if (typeof block.raw === "string" && looksLikeJson(block.raw)) {
        try {
          return { json: JSON.parse(block.raw), jsonLabel: "Raw" };
        } catch {
          return null;
        }
      }
      return null;
    }
  }
}

export function collectEventJsonPayloads(blocks: EventBlock[]): BlockSplitPayload[] {
  return blocks
    .map((block) => getBlockSplitPayload(block))
    .filter((payload): payload is BlockSplitPayload => payload != null);
}

function SessionBlockHumanMeta({ block }: { block: EventBlock }) {
  switch (block.type) {
    case "tool_call":
      return (
        <div className="flex flex-col gap-2">
          <Badge variant="outline">{getBlockLabel(block)}</Badge>
          <p className="text-sm font-medium">{block.name}</p>
          <code className="break-all font-mono text-xs text-muted-foreground">{block.tool_call_id}</code>
        </div>
      );
    case "provider_payload":
      return (
        <div className="flex flex-col gap-2">
          <Badge variant="secondary">{block.kind || "Provider payload"}</Badge>
        </div>
      );
    case "command":
      return (
        <div className="flex flex-col gap-2">
          {block.cwd ? <p className="font-mono text-xs text-muted-foreground">{block.cwd}</p> : null}
          <div className="font-mono text-xs">
            <span className="text-muted-foreground">$ </span>
            <span className="break-words [overflow-wrap:anywhere]">{block.command}</span>
          </div>
        </div>
      );
    case "tool_result":
      return (
        <div className="flex flex-col gap-2">
          {block.is_error ? <Badge variant="destructive">Error</Badge> : null}
          <Badge variant="outline">{getBlockLabel(block)}</Badge>
          <code className="break-all font-mono text-xs text-muted-foreground">{block.tool_call_id}</code>
        </div>
      );
    case "file":
      return (
        <div className="flex flex-col gap-2">
          <Badge variant="outline">{getBlockLabel(block)}</Badge>
          <code className="break-all font-mono text-xs">{block.path}</code>
        </div>
      );
    case "unknown":
      return (
        <div className="flex flex-col gap-2">
          <Badge variant="outline">Unknown block</Badge>
        </div>
      );
    default:
      return null;
  }
}

export function SessionEventHumanBlock({ block }: { block: EventBlock }): ReactNode {
  if (getBlockSplitPayload(block)) {
    return (
      <div className="flex flex-col gap-3">
        <SessionBlockHumanMeta block={block} />
      </div>
    );
  }

  return <SessionBlock block={block} embedded />;
}

export function SessionDetailBlock({ block }: { block: EventBlock }): ReactNode {
  return <SessionEventHumanBlock block={block} />;
}

export { SessionEventJsonColumn } from "@/features/sessions/session-json-panel";
