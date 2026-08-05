import { describe, expect, it } from "vitest";
import { readManagerRouteState, resolveManagerRequest } from "./manager-route-state";

describe("manager route state", () => {
  it("defaults to current workspace and waits until that workspace is known", () => {
    const route = readManagerRouteState(new URLSearchParams());

    expect(route).toEqual({
      view: "sessions",
      scope: "current",
      workspace: null,
      providers: [],
      providerSelection: "all",
      search: "",
      sort: "recent",
      page: 1,
      pageSize: 20,
    });
    expect(resolveManagerRequest(route, null)).toMatchObject({
      enabled: false,
      workspace: null,
    });
    expect(resolveManagerRequest(route, "/work/current")).toEqual({
      enabled: true,
      workspace: "/work/current",
      page: 1,
      pageSize: 20,
      listFilter: {
        providers: [],
        workspace: "/work/current",
        sort: "recent",
        limit: 20,
        offset: 0,
      },
      statsFilter: {
        providers: [],
        workspace: "/work/current",
      },
    });
  });

  it("uses all workspaces only when scope=all", () => {
    const route = readManagerRouteState(new URLSearchParams("scope=all"));

    expect(resolveManagerRequest(route, "/work/current")).toEqual({
      enabled: true,
      workspace: null,
      page: 1,
      pageSize: 20,
      listFilter: {
        providers: [],
        workspace: undefined,
        sort: "recent",
        limit: 20,
        offset: 0,
      },
      statsFilter: {
        providers: [],
        workspace: undefined,
      },
    });
  });

  it("lets an explicit workspace override scope", () => {
    const route = readManagerRouteState(
      new URLSearchParams("scope=all&workspace=%2Fwork%2Fdeep-link"),
    );

    expect(resolveManagerRequest(route, "/work/current")).toMatchObject({
      enabled: true,
      workspace: "/work/deep-link",
      listFilter: { workspace: "/work/deep-link" },
    });
  });

  it("reads plural providers and all public URL state without a singular fallback", () => {
    const route = readManagerRouteState(
      new URLSearchParams(
        "view=workspaces&provider=ignored&providers=codex%2Cclaude%2Ccodex&q=memory&sort=title&page=2&pageSize=50",
      ),
    );

    expect(route).toEqual({
      view: "workspaces",
      scope: "current",
      workspace: null,
      providers: ["codex", "claude"],
      providerSelection: "custom",
      search: "memory",
      sort: "title",
      page: 2,
      pageSize: 50,
    });

    expect(resolveManagerRequest(route, "/work/current").listFilter).toMatchObject({
      search: "memory",
      limit: 50,
      offset: 50,
    });
  });

  it("reads providers=none as an explicit empty selection", () => {
    const route = readManagerRouteState(new URLSearchParams("providers=none"));

    expect(route.providerSelection).toBe("none");
    expect(route.providers).toEqual([]);
  });

  it("keeps stats independent from search, sort, and pagination", () => {
    const route = readManagerRouteState(
      new URLSearchParams(
        "view=workspaces&scope=all&providers=codex&q=memory&sort=sessions&page=3&pageSize=50",
      ),
    );

    const request = resolveManagerRequest(route, "/work/current");

    expect(request.statsFilter).toEqual({
      providers: ["codex"],
      workspace: undefined,
    });
    expect(request.listFilter).toEqual({
      providers: ["codex"],
      workspace: undefined,
      search: "memory",
      sort: "sessions",
      limit: 50,
      offset: 100,
    });
  });

  it("supports workspace session-count sorting without leaking it into sessions", () => {
    expect(
      readManagerRouteState(new URLSearchParams("view=workspaces&sort=sessions")).sort,
    ).toBe("sessions");
    expect(readManagerRouteState(new URLSearchParams("sort=sessions")).sort).toBe(
      "recent",
    );
  });
});
