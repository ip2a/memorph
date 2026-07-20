import { describe, expect, it } from "vitest";
import {
  clampManagerPageSize,
  formatManagerListSummary,
  managerPageRange,
  managerTotalPages,
  readManagerPage,
} from "./manager-pagination";

describe("manager pagination", () => {
  it("clamps page size to supported values", () => {
    expect(clampManagerPageSize(70)).toBe(70);
    expect(clampManagerPageSize(999)).toBe(20);
    expect(clampManagerPageSize(null)).toBe(20);
  });

  it("reads page from URL params", () => {
    expect(readManagerPage(new URLSearchParams("page=3"))).toBe(3);
    expect(readManagerPage(new URLSearchParams("page=0"))).toBe(1);
  });

  it("formats page ranges and totals", () => {
    expect(managerTotalPages(45, 20)).toBe(3);
    expect(managerPageRange(2, 20, 45)).toEqual({ from: 21, to: 40 });
    expect(formatManagerListSummary(2, 20, 45)).toBe("21–40 of 45");
  });
});
