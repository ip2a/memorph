import { describe, expect, it } from "vitest";
import {
  detectSessionContentKind,
  looksLikeJson,
  looksLikeLogOutput,
  looksLikeMarkdown,
  parseLogOutput,
} from "@/lib/format-content";

describe("looksLikeJson", () => {
  it("detects valid JSON objects and arrays", () => {
    expect(looksLikeJson('{"a":1}')).toBe(true);
    expect(looksLikeJson("[1, 2]")).toBe(true);
  });

  it("rejects plain text and markdown", () => {
    expect(looksLikeJson("hello")).toBe(false);
    expect(looksLikeJson("# Title")).toBe(false);
  });
});

describe("looksLikeMarkdown", () => {
  it("detects common markdown patterns", () => {
    expect(looksLikeMarkdown("# Heading")).toBe(true);
    expect(looksLikeMarkdown("- item one")).toBe(true);
    expect(looksLikeMarkdown("**bold** text")).toBe(true);
    expect(looksLikeMarkdown("```ts\nconst x = 1\n```")).toBe(true);
    expect(looksLikeMarkdown("[link](https://example.com)")).toBe(true);
  });

  it("rejects log output that contains markdown-like fragments", () => {
    expect(looksLikeMarkdown("Line 204: some text\n### not a heading")).toBe(false);
  });
});

describe("looksLikeLogOutput", () => {
  it("detects grep-style line prefixes", () => {
    expect(looksLikeLogOutput("Line 204: memorph Manager")).toBe(true);
  });
});

describe("parseLogOutput", () => {
  it("strips Line prefixes and keeps content", () => {
    expect(parseLogOutput("Line 204: memorph Manager\nLine 523: next hit")).toEqual([
      { kind: "match", text: "memorph Manager" },
      { kind: "match", text: "next hit" },
    ]);
  });

  it("classifies standalone paths", () => {
    expect(parseLogOutput("/Volumes/data0/project/README.md")).toEqual([
      { kind: "path", text: "/Volumes/data0/project/README.md" },
    ]);
  });
});

describe("detectSessionContentKind", () => {
  it("prefers json over markdown for structured payloads", () => {
    expect(detectSessionContentKind('{"title":"# not markdown"}')).toBe("json");
  });

  it("classifies markdown, logs, and plain text", () => {
    expect(detectSessionContentKind("## Notes\n\n- one")).toBe("markdown");
    expect(detectSessionContentKind("Line 204: hit")).toBe("log");
    expect(detectSessionContentKind("Hello there.")).toBe("text");
  });
});
