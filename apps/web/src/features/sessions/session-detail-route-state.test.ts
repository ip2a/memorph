import { describe, expect, it } from "vitest";
import {
  buildSessionEventQuery,
  clampSessionEventPageSize,
  sessionEventTotalPages,
} from "@/features/sessions/session-detail-pagination";
import {
  readSessionDetailRouteState,
  writeSessionDetailRouteState,
} from "@/features/sessions/session-detail-route-state";

describe("session detail pagination", () => {
  it("clamps page size to supported values", () => {
    expect(clampSessionEventPageSize(50)).toBe(50);
    expect(clampSessionEventPageSize(999)).toBe(20);
  });

  it("builds API query from page state", () => {
    expect(buildSessionEventQuery(2, 20)).toEqual({
      event_offset: 20,
      event_limit: 20,
    });
    expect(buildSessionEventQuery(1, 20, "needle")).toEqual({
      event_offset: 0,
      event_limit: 20,
      event_search: "needle",
    });
    expect(buildSessionEventQuery(1, 20, "", "desc")).toEqual({
      event_offset: 0,
      event_limit: 20,
      event_order: "desc",
    });
  });

  it("computes total pages from event count", () => {
    expect(sessionEventTotalPages(45, 20)).toBe(3);
  });
});

describe("session detail route state", () => {
  it("reads page and page size from URL params", () => {
    expect(readSessionDetailRouteState(new URLSearchParams("page=3&pageSize=50"))).toEqual({
      page: 3,
      pageSize: 50,
      eventSearch: "",
      eventOrder: "asc",
    });
    expect(readSessionDetailRouteState(new URLSearchParams("q=hello&order=desc"))).toEqual({
      page: 1,
      pageSize: 20,
      eventSearch: "hello",
      eventOrder: "desc",
    });
  });

  it("writes page and page size back to URL params", () => {
    const next = writeSessionDetailRouteState(new URLSearchParams(), {
      page: 2,
      pageSize: 50,
    });
    expect(next.toString()).toBe("page=2&pageSize=50");
  });

  it("writes search query to URL params", () => {
    const next = writeSessionDetailRouteState(new URLSearchParams(), {
      eventSearch: "tool call",
      page: 1,
    });
    expect(next.toString()).toBe("q=tool+call");
  });

  it("writes desc order to URL params", () => {
    const next = writeSessionDetailRouteState(new URLSearchParams(), {
      eventOrder: "desc",
    });
    expect(next.toString()).toBe("order=desc");
  });
});
