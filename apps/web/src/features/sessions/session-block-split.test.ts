import { describe, expect, it } from "vitest";
import { collectEventJsonPayloads, getBlockSplitPayload } from "@/features/sessions/session-block-split";

describe("getBlockSplitPayload", () => {
  it("splits structured tool calls", () => {
    expect(
      getBlockSplitPayload({
        type: "tool_call",
        tool_call_id: "call_1",
        name: "grep",
        input: { pattern: "foo" },
      }),
    ).toEqual({
      json: { tool_call_id: "call_1", name: "grep", input: { pattern: "foo" } },
      jsonLabel: "Request",
    });
  });

  it("keeps markdown text full width", () => {
    expect(getBlockSplitPayload({ type: "text", text: "## Hello" })).toBeNull();
  });

  it("splits JSON tool results", () => {
    expect(
      getBlockSplitPayload({
        type: "tool_result",
        tool_call_id: "call_1",
        content: '{"ok":true}',
      }),
    ).toEqual({
      json: { ok: true },
      jsonLabel: "Response",
    });
  });

  it("collects all JSON payloads for an event", () => {
    expect(
      collectEventJsonPayloads([
        {
          type: "tool_call",
          tool_call_id: "call_1",
          name: "grep",
          input: {},
        },
        { type: "text", text: "hello" },
      ]),
    ).toHaveLength(1);
  });
});
