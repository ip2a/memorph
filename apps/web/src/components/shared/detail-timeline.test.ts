// @vitest-environment jsdom

import { describe, expect, it } from "vitest";
import { eventMinimapClassName, getScrollOffset } from "@/components/shared/detail-timeline";

describe("getScrollOffset", () => {
  it("returns element offset within a scroll container", () => {
    const scrollRoot = document.createElement("div");
    const content = document.createElement("div");
    const target = document.createElement("div");

    Object.defineProperty(scrollRoot, "scrollTop", { value: 40, writable: true });
    scrollRoot.getBoundingClientRect = () =>
      ({
        top: 100,
        left: 0,
        right: 0,
        bottom: 0,
        width: 0,
        height: 0,
        x: 0,
        y: 100,
        toJSON: () => ({}),
      }) as DOMRect;
    target.getBoundingClientRect = () =>
      ({
        top: 260,
        left: 0,
        right: 0,
        bottom: 0,
        width: 0,
        height: 0,
        x: 0,
        y: 260,
        toJSON: () => ({}),
      }) as DOMRect;

    scrollRoot.append(content);
    content.append(target);

    expect(getScrollOffset(target, scrollRoot)).toBe(200);
  });
});

describe("eventMinimapClassName", () => {
  it("maps known roles to stable class tokens", () => {
    expect(eventMinimapClassName("user", "message")).toContain("bg-[#b8c5d4]");
    expect(eventMinimapClassName("assistant", "message")).toContain("bg-[#d4d3cb]");
    expect(eventMinimapClassName("tool", "tool_result")).toContain("bg-[#d9c48a]");
  });

  it("falls back to tool styling for action-like kinds", () => {
    expect(eventMinimapClassName("unknown", "action")).toContain("bg-[#d9c48a]");
    expect(eventMinimapClassName("unknown", "tool_call")).toContain("bg-[#d9c48a]");
  });
});
