import { getBlockLabel, type SessionBlockTag } from "@/features/sessions/session-block-utils";
import type { EventBlock } from "@/lib/types";
import { translate, type I18nKey } from "@/lib/i18n-core";
type Translator = (key: I18nKey, vars?: Record<string, string | number | null | undefined>) => string;

const CHAIN_BLOCK_TYPES = new Set<EventBlock["type"]>(["thinking", "tool_call", "tool_result"]);

export type EventBlockSegment =
  | { kind: "chain"; blocks: EventBlock[] }
  | { kind: "block"; block: EventBlock };

export type ChainStepKind = "thinking" | "tool_call" | "tool_result";

export type ChainStep = {
  id: string;
  kind: ChainStepKind;
  label: string;
  description?: string;
  block: EventBlock;
  /** For thinking blocks split into multiple paragraphs. */
  thinkingText?: string;
};

export function isChainBlock(block: EventBlock): boolean {
  return CHAIN_BLOCK_TYPES.has(block.type);
}

/** Header chips: one "Chain of thought" tag instead of per-chain block tags. */
export function getEventHeaderTags(blocks: EventBlock[] | undefined, t?: Translator): SessionBlockTag[] {
  const list = blocks ?? [];
  const tags: SessionBlockTag[] = [];

  if (list.some(isChainBlock)) {
    tags.push({
      type: "chain",
      label: t?.("chainOfThought") ?? translate("en", "chainOfThought"),
    });
  }

  for (const block of list) {
    if (isChainBlock(block)) continue;
    const label = getBlockLabel(block, t);
    if (label) tags.push({ type: block.type, label });
  }

  return tags;
}

export function segmentEventBlocks(blocks: EventBlock[]): EventBlockSegment[] {
  const segments: EventBlockSegment[] = [];
  let chain: EventBlock[] = [];

  const flushChain = () => {
    if (chain.length === 0) return;
    segments.push({ kind: "chain", blocks: [...chain] });
    chain = [];
  };

  for (const block of blocks) {
    if (isChainBlock(block)) {
      chain.push(block);
      continue;
    }
    flushChain();
    segments.push({ kind: "block", block });
  }

  flushChain();
  return segments;
}

export function splitThinkingSteps(text: string): string[] {
  const trimmed = text.trim();
  if (!trimmed) return [];

  const numbered = trimmed.split(/\n(?=\d+[.)]\s)/).map((part) => part.trim()).filter(Boolean);
  if (numbered.length > 1) return numbered;

  const paragraphs = trimmed.split(/\n{2,}/).map((part) => part.trim()).filter(Boolean);
  if (paragraphs.length > 1) return paragraphs;

  return [trimmed];
}

export function previewText(value: string, maxLength = 96): string {
  const normalized = value.replace(/\s+/g, " ").trim();
  if (normalized.length <= maxLength) return normalized;
  return `${normalized.slice(0, maxLength - 1)}…`;
}

export function previewToolInput(input: unknown): string | undefined {
  if (input == null) return undefined;
  if (typeof input === "string") {
    const preview = previewText(input, 72);
    return preview || undefined;
  }
  try {
    const preview = previewText(JSON.stringify(input), 72);
    return preview || undefined;
  } catch {
    return undefined;
  }
}

export function blocksToChainSteps(blocks: EventBlock[], t?: Translator): ChainStep[] {
  const label = (key: I18nKey, vars?: Record<string, string | number>) => t?.(key, vars) ?? translate("en", key, vars);
  const steps: ChainStep[] = [];
  let index = 0;

  for (const block of blocks) {
    if (block.type === "thinking") {
      const parts = splitThinkingSteps(block.text);
      if (parts.length <= 1) {
        steps.push({
          id: `thinking-${index}`,
          kind: "thinking",
          label: label("blockThinking"),
          block,
          thinkingText: block.text,
        });
      } else {
        for (const [partIndex, part] of parts.entries()) {
          steps.push({
            id: `thinking-${index}-${partIndex}`,
            kind: "thinking",
            label: parts.length > 1 ? `${label("blockThinking")} ${partIndex + 1}` : label("blockThinking"),
            block,
            thinkingText: part,
          });
        }
      }
      index += 1;
      continue;
    }

    if (block.type === "tool_call") {
      steps.push({
        id: `tool-${block.tool_call_id || index}`,
        kind: "tool_call",
        label: block.name || label("blockToolCall"),
        description: previewToolInput(block.input),
        block,
      });
      index += 1;
      continue;
    }

    if (block.type === "tool_result") {
      steps.push({
        id: `result-${block.tool_call_id || index}`,
        kind: "tool_result",
        label: block.is_error ? label("blockToolError") : label("blockToolResult"),
        block,
      });
      index += 1;
    }
  }

  return steps;
}
