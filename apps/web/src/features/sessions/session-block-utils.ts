import type { EventBlock } from "@/lib/types";
import { translate, type I18nKey } from "@/lib/i18n-core";
type Translator = (key: I18nKey, vars?: Record<string, string | number | null | undefined>) => string;

export function getBlockLabel(block: EventBlock, t?: Translator): string {
  const label = (key: I18nKey, vars?: Record<string, string | number>) => t?.(key, vars) ?? translate("en", key, vars);
  switch (block.type) {
    case "text":
      return "";
    case "thinking":
      return label("blockThinking");
    case "tool_call":
      return block.name ? label("blockTool", { name: block.name }) : label("blockToolCall");
    case "tool_result":
      return label("blockToolResult");
    case "patch":
      return label("blockPatch");
    case "command":
      return label("blockCommand");
    case "command_result":
      return label("blockCommandResult");
    case "file":
      return label("blockFile");
    case "image":
      return label("blockImage");
    case "compressed":
      return label("blockCompressed");
    case "other":
      return label("blockOther");
    default:
      return "";
  }
}

export type SessionBlockTag = {
  type: EventBlock["type"];
  label: string;
};

export function getBlockTags(blocks: EventBlock[] | undefined, t?: Translator): SessionBlockTag[] {
  return (blocks ?? [])
    .map((block) => {
      const label = getBlockLabel(block, t);
      return label ? { type: block.type, label } : null;
    })
    .filter((tag): tag is SessionBlockTag => tag != null);
}

/** Role fill tokens — shared with the session minimap. */
export function eventRoleTagClass(role: string | null | undefined) {
  switch (role) {
    case "user":
      return "border-transparent bg-[#b8c5d4] text-foreground dark:bg-[#4a5f75] dark:text-foreground";
    case "assistant":
      return "border-transparent bg-[#d4d3cb] text-foreground dark:bg-[#5c5b55] dark:text-foreground";
    case "tool":
      return "border-transparent bg-[#d9c48a] text-foreground dark:bg-[#7a6838] dark:text-foreground";
    case "system":
      return "border-transparent bg-muted text-muted-foreground";
    case "developer":
      return "border-transparent bg-[#c8bdd9] text-foreground dark:bg-[#5f5470] dark:text-foreground";
    default:
      return "border-border bg-background text-muted-foreground";
  }
}

/** OASF EventKind: message / action / observation / lifecycle / other */
export function eventKindTagClass(kind: string | null | undefined) {
  switch (kind) {
    case "message":
      return "border-transparent bg-secondary text-secondary-foreground";
    case "action":
      return "border-transparent bg-[#d9c48a]/70 text-foreground dark:bg-[#7a6838]/70";
    case "observation":
      return "border-[#c5d4b8] bg-[#e7efdf] text-foreground dark:border-[#5f7054] dark:bg-[#3f4a38]";
    case "lifecycle":
      return "border-border bg-muted/60 text-muted-foreground";
    case "other":
      return "border-dashed border-border bg-background text-muted-foreground";
    default:
      return "border-border bg-background text-muted-foreground";
  }
}

/** Block content tags (text is omitted — it is the body, not a header chip). */
export function eventBlockTagClass(type: EventBlock["type"] | string) {
  switch (type) {
    case "chain":
    case "thinking":
      return "border-transparent bg-[#c8bdd9]/55 text-foreground dark:bg-[#5f5470]/55";
    case "tool_call":
      return "border-transparent bg-[#d9c48a]/70 text-foreground dark:bg-[#7a6838]/70";
    case "tool_result":
      return "border-[#d9c48a] bg-[#f3ead0] text-foreground dark:border-[#7a6838] dark:bg-[#4a4024]";
    case "command":
      return "border-transparent bg-[#b8c5d4]/70 text-foreground dark:bg-[#4a5f75]/70";
    case "command_result":
      return "border-[#b8c5d4] bg-[#e4ebf2] text-foreground dark:border-[#4a5f75] dark:bg-[#2f3c4a]";
    case "patch":
      return "border-transparent bg-[#c5d4b8]/70 text-foreground dark:bg-[#5f7054]/70";
    case "file":
      return "border-border bg-background text-foreground";
    case "image":
      return "border-border bg-background text-foreground";
    case "compressed":
      return "border-transparent bg-secondary text-secondary-foreground";
    case "other":
      return "border-dashed border-border bg-background text-muted-foreground";
    default:
      return "border-border bg-background text-muted-foreground";
  }
}
