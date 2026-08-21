import { describe, expect, it } from "vitest";
import {
  blocksToChainSteps,
  getEventHeaderTags,
  segmentEventBlocks,
  splitThinkingSteps,
} from "@/features/sessions/session-chain-of-thought-utils";
import type { EventBlock } from "@/lib/types";

describe("splitThinkingSteps", () => {
  it("splits numbered steps", () => {
    expect(splitThinkingSteps("1. Plan\n2. Execute")).toEqual(["1. Plan", "2. Execute"]);
  });

  it("splits paragraphs", () => {
    expect(splitThinkingSteps("First thought.\n\nSecond thought.")).toEqual([
      "First thought.",
      "Second thought.",
    ]);
  });

  it("keeps single blocks intact", () => {
    expect(splitThinkingSteps("One continuous thought.")).toEqual(["One continuous thought."]);
  });
});

describe("segmentEventBlocks", () => {
  it("groups reasoning blocks into a chain segment", () => {
    const blocks: EventBlock[] = [
      { type: "thinking", text: "Need to inspect files." },
      { type: "tool_call", tool_call_id: "t1", name: "Read", input: { path: "a.ts" } },
      { type: "tool_result", tool_call_id: "t1", content: "ok" },
      { type: "text", text: "Here is the answer." },
    ];

    expect(segmentEventBlocks(blocks)).toEqual([
      {
        kind: "chain",
        blocks: blocks.slice(0, 3),
      },
      {
        kind: "block",
        block: blocks[3],
      },
    ]);
  });
});

describe("getEventHeaderTags", () => {
  it("collapses chain blocks into one chain-of-thought tag", () => {
    const blocks: EventBlock[] = [
      { type: "thinking", text: "Plan the change." },
      { type: "tool_call", tool_call_id: "t1", name: "Grep", input: { pattern: "foo" } },
      { type: "text", text: "Final answer." },
    ];

    expect(getEventHeaderTags(blocks)).toEqual([
      { type: "chain", label: "Chain of thought" },
    ]);
  });
});

describe("blocksToChainSteps", () => {
  it("creates one step per reasoning block", () => {
    const blocks: EventBlock[] = [
      { type: "thinking", text: "Plan the change." },
      { type: "tool_call", tool_call_id: "t1", name: "Grep", input: { pattern: "foo" } },
    ];

    const steps = blocksToChainSteps(blocks);
    expect(steps).toHaveLength(2);
    expect(steps[0]?.kind).toBe("thinking");
    expect(steps[1]?.label).toBe("Grep");
  });
});
