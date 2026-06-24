export function createHomeModule({
  state,
  providers,
  ascii,
  t,
  escapeHtml,
  escapeAttr,
  formatDate,
  formatBytes,
  workspaceName,
  render,
}) {
  function shouldAutoCollapseHomeHero() {
    return (
      !!state.home.search.trim() ||
      state.home.sort !== "recent" ||
      state.home.hookFilter !== "all" ||
      state.ui.homeHeroTransientCollapsed ||
      window.innerHeight < 760
    );
  }

  function isHomeHeroCollapsed() {
    if (state.ui.homeHeroMode === "collapsed") return true;
    if (state.ui.homeHeroMode === "expanded") return false;
    return shouldAutoCollapseHomeHero();
  }

  function renderHomeHeroMeta(filteredGroups, totalSessions, shownSessions) {
    return `
      <div class="meta-line">
        <span>${t("sessionsStat")}=${totalSessions}</span>
        <span>${t("terminalAgents")}=${filteredGroups.length}</span>
        <span>${t("shown")}=${shownSessions}</span>
        <span>${t("hookFilter")}=${escapeHtml(activeHookFilterLabel())}</span>
      </div>`;
  }

  function renderHomeHero(filteredGroups, totalSessions, shownSessions) {
    const collapsed = isHomeHeroCollapsed();
    const title = escapeHtml(workspaceName(state.home.workspace) || "memorph");
    const path = escapeHtml(state.home.workspace || "—");
    const pathTitle = escapeAttr(state.home.workspace || "");

    if (collapsed) {
      return `
        <section class="home-hero home-hero-collapsed">
          <div class="home-hero-compact home-hero-compact-action" data-action="set-home-hero-mode" data-mode="expanded" role="button" tabindex="0" title="${escapeAttr(t("expand"))}">
            <div class="home-hero-compact-main">
              <span class="eyebrow">${t("workspace")}</span>
              <strong>${title}</strong>
              <span class="workspace-path home-hero-compact-path" title="${pathTitle}">${path}</span>
            </div>
            ${renderHomeHeroMeta(filteredGroups, totalSessions, shownSessions)}
          </div>
        </section>`;
    }

    return `
      <section class="home-hero">
        <div class="ascii-banner ascii-banner-action" data-action="set-home-hero-mode" data-mode="collapsed" title="${escapeAttr(t("collapse"))}" style="--ascii-banner-color: ${escapeAttr(state.ui.asciiBannerColor)}"><pre>${escapeHtml(ascii)}</pre></div>
        <div class="workspace-hero">
          <p class="eyebrow">${t("workspace")}</p>
          <h1>${title}</h1>
          <button type="button" class="workspace-path" data-action="open-workspace-switch" title="${pathTitle}">
            ${path}
          </button>
          ${renderHomeHeroMeta(filteredGroups, totalSessions, shownSessions)}
        </div>
      </section>`;
  }

  function findSyncRef(providerId, sessionId) {
    const groups = state.home.syncGroups || [];
    const match = groups.find((group) =>
      (group.holdings || []).some((holding) => holding.provider === providerId && holding.session_id === sessionId)
    );
    return match?.id || null;
  }

  function isOpenCodeSubagentTitle(title) {
    if (!title) return false;
    return title.includes("(@") && title.includes(" subagent)");
  }

  function getOrderedProviders() {
    return providers.all();
  }

  function getToolbarProviderCandidates() {
    const visible = providers
      .visible()
      .filter((item) => providers.hasFilter(item, "is_installed") && providers.hasFilter(item, "has_sessions"));
    if (visible.length) return visible;

    const ordered = getOrderedProviders();
    const fallbackIds = ["claude", "codex"];
    const fallback = fallbackIds
      .map((id) => ordered.find((item) => item.provider_id === id))
      .filter(Boolean);
    return fallback.length ? fallback : ordered.slice(0, 2);
  }

  function getVisibleToolbarProviders() {
    const candidates = getToolbarProviderCandidates();
    if (!candidates.length) return [];

    const minVisible = state.home.providers.length ? 1 : Math.min(2, candidates.length);
    const storedCount = Number(state.ui.homeProviderVisibleCount || 0);
    const visibleCount = storedCount > 0 ? storedCount : minVisible;
    return candidates.slice(0, Math.max(minVisible, Math.min(visibleCount, candidates.length)));
  }

  let homeProviderLayoutFrame = 0;

  function scheduleHomeProviderLayout() {
    window.cancelAnimationFrame(homeProviderLayoutFrame);
    homeProviderLayoutFrame = window.requestAnimationFrame(updateHomeProviderLayout);
  }

  function updateHomeProviderLayout() {
    if (state.route.name !== "home") return;

    const strip = document.querySelector(".home-provider-strip");
    if (!strip) return;

    const candidates = getToolbarProviderCandidates();
    if (!candidates.length) return;

    const moreWidth = measureToolbarControlWidth(t("more"), true);
    const available = strip.clientWidth;
    const minVisible = state.home.providers.length ? 1 : Math.min(2, candidates.length);

    let used = moreWidth;
    let visible = 0;
    for (const item of candidates) {
      const pillWidth = measureToolbarControlWidth(item.display_name, false);
      const nextUsed = used + (visible ? 8 : 0) + pillWidth;
      if (nextUsed <= available || visible < minVisible) {
        used = nextUsed;
        visible += 1;
        continue;
      }
      break;
    }

    const nextCount = Math.max(minVisible, Math.min(visible, candidates.length));
    if (state.ui.homeProviderVisibleCount !== nextCount) {
      state.ui.homeProviderVisibleCount = nextCount;
      render();
    }
  }

  function measureToolbarControlWidth(label, more = false) {
    const root = document.createElement(more ? "button" : "label");
    root.style.position = "fixed";
    root.style.left = "-10000px";
    root.style.top = "0";
    root.style.visibility = "hidden";
    root.style.pointerEvents = "none";

    if (more) {
      root.className = "agent-more-button";
      root.textContent = label;
    } else {
      root.className = "agent-pill";
      root.innerHTML = `<span>${escapeHtml(label)}</span>`;
    }

    document.body.append(root);
    const width = Math.ceil(root.getBoundingClientRect().width);
    root.remove();
    return width;
  }

  function getPrimaryProviders() {
    return providers.visible().slice(0, 3);
  }

  function getFoldedProviders() {
    return providers.visible().slice(3);
  }

  function getDefaultSwitchTarget(sourceId) {
    const ordered = getOrderedProviders().filter((item) => item.provider_id !== sourceId);
    if (!ordered.length) return "";
    if (sourceId === "codex") {
      const claude = ordered.find((item) => item.provider_id === "claude");
      if (claude) return claude.id;
    }
    return ordered[0].id;
  }

  function sortSessionGroupsByDisplay(groups) {
    const order = getOrderedProviders().map((item) => item.provider_id);
    const indexMap = new Map(order.map((id, index) => [id, index]));
    return [...groups].sort((left, right) => {
      const leftIndex = indexMap.has(left.provider_id) ? indexMap.get(left.provider_id) : Number.MAX_SAFE_INTEGER;
      const rightIndex = indexMap.has(right.provider_id) ? indexMap.get(right.provider_id) : Number.MAX_SAFE_INTEGER;
      if (leftIndex !== rightIndex) return leftIndex - rightIndex;
      return providers.displayName(left.provider_id).localeCompare(providers.displayName(right.provider_id));
    });
  }

  function filterAndSortGroups(groups) {
    const q = state.home.search.trim().toLowerCase();
    const visible = Math.max(1, Number(state.home.visible || 12));
    const showSubagents = state.meta.settings.show_opencode_subagents;
    return groups
      .map((group) => {
        let sessions = [...group.sessions];
        if (!showSubagents && group.provider_id === "opencode") {
          sessions = sessions.filter((item) => !isOpenCodeSubagentTitle(item.title));
        }
        if (q) {
          sessions = sessions.filter((item) =>
            [item.session_id, item.title, item.native_title, item.project_dir].some((value) =>
              String(value || "")
                .toLowerCase()
                .includes(q)
              )
          );
        }
        sessions.sort((left, right) => {
          if (state.home.sort === "hook_attention") {
            const severity = hookAttentionPriority(left) - hookAttentionPriority(right);
            if (severity !== 0) return severity;
            return compareRecentThenTitle(left, right);
          }
          if (state.home.sort === "title") {
            return compareTitleThenRecent(left, right);
          }
          return compareRecentThenTitle(left, right);
        });
        return {
          ...group,
          total_sessions: sessions.length,
          shown_sessions: Math.min(sessions.length, visible),
          sessions: sessions.slice(0, visible),
        };
      })
      .filter((group) => group.sessions.length > 0);
  }

  function compareRecentThenTitle(left, right) {
    const timeDiff = (right.last_active_at || 0) - (left.last_active_at || 0);
    if (timeDiff !== 0) return timeDiff;
    return sessionDisplayTitle(left).localeCompare(sessionDisplayTitle(right));
  }

  function compareTitleThenRecent(left, right) {
    const titleCompare = sessionDisplayTitle(left).localeCompare(sessionDisplayTitle(right));
    if (titleCompare !== 0) return titleCompare;
    return (right.last_active_at || 0) - (left.last_active_at || 0);
  }

  function sessionDisplayTitle(item) {
    return String(item.display_title || item.title || item.session_id || "");
  }

  function hookAttentionPriority(item) {
    const kind = item.hook_diagnosis?.kind || "";
    switch (kind) {
      case "hook_needs_attention":
        return 0;
      case "no_session_match":
        return 1;
      case "hook_not_installed":
        return 2;
      case "no_active_runtime":
        return 3;
      case "no_events_yet":
        return 4;
      case "weakly_linked":
        return 5;
      case "linked":
        return 6;
      case "hook_unsupported":
        return 7;
      default:
        return 8;
    }
  }

  function activeHookFilterLabel() {
    switch (state.home.hookFilter) {
      case "attention":
        return t("hookFilterAttention");
      case "weak":
        return t("hookFilterWeak");
      case "runtime":
        return t("hookFilterRuntime");
      case "linked":
        return t("hookFilterLinked");
      case "no_hook":
        return t("hookFilterNoHook");
      case "no_match":
        return t("hookFilterNoMatch");
      default:
        return t("allHookStates");
    }
  }

  function renderSessionRow(item) {
    const syncRef = findSyncRef(item.provider_id, item.session_id);
    const buttons = state.meta.settings.home_buttons;
    const syncAction = syncRef
      ? `<a class="button" href="/sync/${syncRef}" data-nav="/sync/${syncRef}">${t("openSync")}</a>`
      : `<button type="button" data-action="open-sync-create" data-provider="${escapeAttr(item.provider_id)}" data-session-id="${escapeAttr(item.session_id)}" data-title="${escapeAttr(item.title || "")}">${t("syncAction")}</button>`;
    const hookRuntime = renderHookRuntimeBadge(item.hook_runtime_summary, item.hook_diagnosis);
    return `
      <article class="session-row">
        <div class="session-row-main">
          <div class="session-info">
            <div class="session-title-line">
              <a class="session-title session-title-link" href="/sessions/${encodeURIComponent(item.provider_id)}/${encodeURIComponent(item.session_id)}" data-nav="/sessions/${encodeURIComponent(item.provider_id)}/${encodeURIComponent(item.session_id)}">
                ${escapeHtml(item.title || item.session_id)}
              </a>
              <span class="session-workspace">${escapeHtml(item.project_dir || "—")}</span>
            </div>
            <div class="session-meta-bar">
              <span class="session-id-pill">${escapeHtml(item.session_id)}</span>
              ${syncRef ? `<a class="sync-badge" href="/sync/${syncRef}" data-nav="/sync/${syncRef}">${t("activeSync")}</a>` : ""}
              ${hookRuntime}
              <span class="meta-dot">·</span>
              <span class="meta-item" title="${escapeAttr(t("lastActiveAt"))}">${escapeHtml(formatDate(item.last_active_at))}</span>
              <span class="meta-dot">·</span>
              <span class="meta-item" title="${escapeAttr(t("messageCount"))}">${escapeHtml(String(item.message_count ?? "—"))}</span>
              <span class="meta-dot">·</span>
              <span class="meta-item" title="${escapeAttr(t("size"))}">${escapeHtml(formatBytes(item.size_bytes))}</span>
            </div>
          </div>
          <div class="row-actions">
          ${buttons.view ? `<a class="button" href="/sessions/${encodeURIComponent(item.provider_id)}/${encodeURIComponent(item.session_id)}" data-nav="/sessions/${encodeURIComponent(item.provider_id)}/${encodeURIComponent(item.session_id)}">${t("view")}</a>` : ""}
          ${buttons.switch ? `<button type="button" data-action="open-switch" data-provider="${escapeAttr(item.provider_id)}" data-session-id="${escapeAttr(item.session_id)}" data-workspace="${escapeAttr(item.project_dir || state.home.workspace)}" data-title="${escapeAttr(item.title || item.session_id)}">${t("switch")}</button>` : ""}
          ${buttons.export ? `<button type="button" data-action="open-export" data-provider="${escapeAttr(item.provider_id)}" data-session-id="${escapeAttr(item.session_id)}">${t("export")}</button>` : ""}
          ${buttons.sync ? syncAction : ""}
          <button type="button" data-action="open-rename" data-provider="${escapeAttr(item.provider_id)}" data-session-id="${escapeAttr(item.session_id)}" data-title="${escapeAttr(item.title || "")}">${t("rename")}</button>
          ${buttons.delete ? `<button type="button" class="danger" data-action="open-delete" data-provider="${escapeAttr(item.provider_id)}" data-session-id="${escapeAttr(item.session_id)}">${t("remove")}</button>` : ""}
          </div>
        </div>
      </article>`;
  }

  function renderHookRuntimeBadge(summary, diagnosis) {
    if (!summary && !diagnosis) return "";
    if (!summary) {
      const label = diagnosis.kind === "hook_not_installed"
        ? "no-hook"
        : diagnosis.kind === "hook_needs_attention"
          ? "repair"
          : diagnosis.kind === "no_session_match"
            ? "no-match"
            : diagnosis.kind === "no_active_runtime"
              ? "offline"
              : diagnosis.kind === "no_events_yet"
                ? "no-events"
                : diagnosis.kind === "hook_unsupported"
                  ? "n/a"
                  : diagnosis.kind || "hook";
      const title = [
        `diagnosis=${diagnosis.kind || "unknown"}`,
        diagnosis.provider_status ? `status=${diagnosis.provider_status}` : "",
        diagnosis.provider_runtime_sessions ? `provider_rt=${diagnosis.provider_runtime_sessions}` : "",
        diagnosis.message || "",
      ]
        .filter(Boolean)
        .join(" · ");
      return `<span class="status-pill" title="${escapeAttr(title)}">${escapeHtml(label)}</span>`;
    }
    const stateLabel = summary.has_pending_permission
      ? "perm"
      : summary.has_pending_question
        ? "question"
        : summary.current_tool_name || summary.status || "hook";
    const linked = Number(summary.linked_sessions || 0);
    const waiting = Number(summary.waiting_sessions || 0);
    const title = [
      `hook=${stateLabel}`,
      linked > 1 ? `linked=${linked}` : "",
      waiting > 0 ? `waiting=${waiting}` : "",
      summary.matched_by ? `matched_by=${summary.matched_by}` : "",
      summary.confidence ? `confidence=${summary.confidence}` : "",
      diagnosis?.kind ? `diagnosis=${diagnosis.kind}` : "",
      summary.last_event_at ? `last=${formatDate(summary.last_event_at)}` : "",
    ]
      .filter(Boolean)
      .join(" · ");
    const prefix = linked > 1 ? `${linked} ` : "";
    return `<span class="status-pill" title="${escapeAttr(title)}">${escapeHtml(`${prefix}${stateLabel}`)}</span>`;
  }

  function renderHomeGroups(groups) {
    if (!groups.length) {
      return `<div class="empty-state">${t("emptySessions")}</div>`;
    }
    return `
      ${groups
        .map(
          (group) => `
        <details class="provider-section" open>
          <summary>
            <span>${escapeHtml(providers.displayName(group.provider_id))}</span>
            <span>${group.shown_sessions || group.sessions.length}/${group.total_sessions || group.sessions.length}</span>
          </summary>
          <div class="session-list">
            ${group.sessions.map((item) => renderSessionRow(item)).join("")}
          </div>
        </details>`
        )
        .join("")}`;
  }

  function renderProviderPicker() {
    const primary = getVisibleToolbarProviders();
    const primaryMarkup = primary
      .map((item) => {
        const checked = state.home.providers.includes(item.provider_id);
        return `
          <label class="agent-pill">
            <input data-role="provider-toggle" type="checkbox" value="${escapeAttr(item.provider_id)}" ${checked ? "checked" : ""}>
            <span>${escapeHtml(item.display_name)}</span>
          </label>`;
      })
      .join("");

    return `
      <div class="agent-picker-shell home-provider-strip">
        <div class="agent-picker">${primaryMarkup}</div>
        <button type="button" class="agent-more-button" data-action="open-agent-filter">${t("more")}</button>
      </div>`;
  }

  function renderWorkspacePicker() {
    return `
      <label class="field field-wide">
        <span>${t("workspacePath")}</span>
        <div class="workspace-combo">
          <input name="workspace" list="known-workspaces" value="${escapeAttr(state.home.workspace || "")}" placeholder="${escapeAttr(
            state.meta?.workspaces?.[0]?.path || ""
          )}">
          <button type="button" class="ghost" data-action="browse-folder" data-target-field="workspace">${t("browse")}</button>
          <button type="button" class="ghost" data-action="open-workspace-history">${t("history")}</button>
          <button type="submit" class="invert">${t("go")}</button>
        </div>
        <small class="muted">${t("workspaceFieldHint")}</small>
      </label>`;
  }

  function renderHome() {
    const filteredGroups = sortSessionGroupsByDisplay(filterAndSortGroups(state.home.groups));
    const totalSessions = filteredGroups.reduce((sum, group) => sum + (group.total_sessions || group.sessions.length), 0);
    const shownSessions = filteredGroups.reduce((sum, group) => sum + group.sessions.length, 0);
    return `
      <div class="page-home">
        ${renderHomeHero(filteredGroups, totalSessions, shownSessions)}
        <section class="section-panel session-results">
          <div class="section-heading home-list-head">
            <div>
              <strong>${t("recentSessions")}</strong>
              <small>${t("filters")}</small>
            </div>
            <div class="home-list-controls">
              ${renderProviderPicker()}
              <form class="session-search" id="home-search-form" data-submit="home-search">
                <input data-role="home-search" name="search" value="${escapeAttr(state.home.search)}" placeholder="${escapeAttr(
                  t("searchPlaceholder")
                )}">
              </form>
              <button type="button" data-action="open-sort-options">${t("sort")}</button>
              <button type="submit" form="home-search-form">${t("filters")}</button>
            </div>
          </div>
          ${renderHomeGroups(filteredGroups)}
        </section>
      </div>`;
  }

  function providerOptions(skipId = "", selectedId = "") {
    return getOrderedProviders()
      .filter((item) => item.provider_id !== skipId)
      .map(
        (item) =>
          `<option value="${escapeAttr(item.provider_id)}"${item.provider_id === selectedId ? " selected" : ""}>${escapeHtml(item.display_name)}</option>`
      )
      .join("");
  }

  return {
    filterAndSortGroups,
    findSyncRef,
    getDefaultSwitchTarget,
    getFoldedProviders,
    getOrderedProviders,
    providerOptions,
    renderHome,
    renderProviderPicker,
    renderSessionRow,
    renderWorkspacePicker,
    scheduleHomeProviderLayout,
    sortSessionGroupsByDisplay,
  };
}
