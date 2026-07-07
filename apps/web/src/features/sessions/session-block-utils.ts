import type { EventBlock } from "@/lib/types";

export function getBlockLabel(block: EventBlock): string {
  switch (block.type) {
    case "text":
      return "";
    case "thinking":
      return "Thinking";
    case "tool_call":
      return `Tool: ${block.name || ""}`.replace(/:\s$/, "");
    case "tool_result":
      return "Tool Result";
    case "patch":
      return "Patch";
    case "command":
      return "Command";
    case "command_result":
      return "Command Result";
    case "file":
      return "File";
    case "image":
      return "Image";
    case "provider_payload":
      return block.kind || "payload";
    case "compressed":
      return "Compressed";
    case "unknown":
      return "Details";
    default:
      return "";
  }
}
