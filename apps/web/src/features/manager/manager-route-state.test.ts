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
      search: "",
      sort: "recent",
    });
    expect(resolveManagerRequest(route, null)).toMatchObject({
      enabled: false,
      workspace: null,
    });
    expect(resolveManagerRequest(route, "/work/current")).toEqual({
      enabled: true,
      workspace: "/work/current",
      filter: {
        providers: [],
        workspace: "/work/current",
        sort: "recent",
        limit: 100,
      },
    });
  });

  it("uses all workspaces only when scope=all", () => {
    const route = readManagerRouteState(new URLSearchParams("scope=all"));

    expect(resolveManagerRequest(route, "/work/current")).toEqual({
      enabled: true,
      workspace: null,
      filter: {
        providers: [],
        workspace: undefined,
        sort: "recent",
        limit: 100,
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
      filter: { workspace: "/work/deep-link" },
    });
  });

  it("reads plural providers and all public URL state without a singular fallback", () => {
    const route = readManagerRouteState(
      new URLSearchParams(
        "view=workspaces&provider=ignored&providers=codex%2Cclaude%2Ccodex&q=memory&sort=title",
      ),
    );

    expect(route).toEqual({
      view: "workspaces",
      scope: "current",
      workspace: null,
      providers: ["codex", "claude"],
      search: "memory",
      sort: "title",
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
