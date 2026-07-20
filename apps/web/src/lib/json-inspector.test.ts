import { describe, expect, it } from "vitest";
import { buildJsonFieldList, formatJsonDocument, normalizeJsonDocument } from "@/lib/json-inspector";

describe("json inspector", () => {
  it("normalizes JSON strings", () => {
    expect(normalizeJsonDocument('{"name":"glob"}')).toEqual({
      ok: true,
      data: { name: "glob" },
    });
  });

  it("builds nested field metadata", () => {
    const fields = buildJsonFieldList({
      tool_call_id: "abc",
      input: { path: "src" },
    });

    expect(fields.map((field) => field.key)).toEqual(["tool_call_id", "input", "path"]);
    expect(fields.find((field) => field.key === "input")?.type).toBe("object");
    expect(fields.find((field) => field.key === "path")?.depth).toBe(1);
  });

  it("formats pretty JSON", () => {
    expect(formatJsonDocument({ a: 1 })).toBe('{\n  "a": 1\n}');
  });
});
