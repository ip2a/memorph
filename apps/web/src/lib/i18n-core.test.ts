import { describe, expect, it } from "vitest";
import { dictionaries } from "@/lib/i18n-core";

describe("i18n dictionaries", () => {
  it("keeps Chinese and English keys aligned and non-empty", () => {
    expect(Object.keys(dictionaries.en)).toEqual(Object.keys(dictionaries.zh));
    expect(Object.values(dictionaries.zh).every(Boolean)).toBe(true);
    expect(Object.values(dictionaries.en).every(Boolean)).toBe(true);
  });
});
