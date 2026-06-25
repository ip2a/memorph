export function createHooksCenterModule({
  state,
  providers,
  t,
  escapeHtml,
  escapeAttr,
  formatDate,
  formatValue,
  renderMetaLine,
  workspaceName,
}) {
  function renderHooksCenterPage() {
    const overview = state.hooks.overview;
    if (!overview) {
      return `<div class="empty-state">${t("loading")}</div>`;
    }

    return `
      <div class="manager-page-layout agent-management-page-layout">
        <section class="section-panel manager-control-panel agent-provider-panel">
          ${renderHooksOverviewSummary(overview)}
          ${renderHooksProviderList(overview.providers || [])}
        </section>

        <section class="section-panel manager-result-panel agent-provider-detail-panel">
          ${
            state.hooks.selectedProvider && state.hooks.providerDetail
              ? renderProviderDetail(state.hooks.providerDetail)
              : `<div class="empty-state">${t("hookSelectProvider")}</div>`
          }
        </section>
      </div>
    `;
  }

  function renderHooksOverviewSummary(overview) {
    const server = overview.server || {};
    const title = escapeHtml(workspaceName(state.home.workspace) || "memorph");
    const path = escapeHtml(state.home.workspace || "—");
    const pathTitle = escapeAttr(state.home.workspace || "");
    return `
      <section class="manager-workspace-summary hook-workspace-summary">
        <div class="hook-workspace-head">
          <p class="eyebrow">${t("workspace")}</p>
          <strong>${title}</strong>
          <button type="button" class="workspace-path" data-action="open-workspace-switch" title="${pathTitle}">${path}</button>
        </div>
      </section>`;
  }

  function renderHooksProviderList(providerList) {
    const visible = providerList
      .filter((item) => !providers.isHiddenGlobal(item.provider_id))
      .sort((left, right) => {
        const leftInstalled = providers.isInstalled(left.provider_id);
        const rightInstalled = providers.isInstalled(right.provider_id);
        if (leftInstalled === rightInstalled) return 0;
        return leftInstalled ? -1 : 1;
      });
    if (!visible.length) {
      return `<div class="empty-state">${t("noProviders")}</div>`;
    }
    return `
      <div class="manager-list agent-provider-list hook-provider-list">
        ${visible.map(renderHooksProviderListItem).join("")}
      </div>`;
  }

  function renderHooksProviderListItem(provider) {
    const providerId = provider.provider_id || "";
    const selected = state.hooks.selectedProvider === providerId;
    const installed = providers.isInstalled(providerId);
    return `
      <button
        type="button"
        class="agent-provider-item ${selected ? "is-active" : ""}"
        data-action="select-hook-provider"
        data-provider="${escapeAttr(providerId)}"
      >
        <span class="agent-provider-head">
          <strong class="agent-provider-name">${escapeHtml(providerDisplayName(provider))}</strong>
          <span class="settings-provider-status ${installed ? "is-installed" : "is-missing"}">${escapeHtml(installed ? t("installed") : t("notDetected"))}</span>
        </span>
      </button>`;
  }

  function providerDisplayName(provider) {
    return providers.displayName(provider);
  }

  function renderProviderDetail(detail) {
    const provider = detail.provider;
    const providerId = provider?.provider_id || "";
    if (!providerId) {
      return `<div class="empty-state">${t("hookSelectProvider")}</div>`;
    }

    const hook = provider.hook || {};
    const diagnosis = provider.hook_diagnosis || {};
    const profile = provider.hook_profile || {};
    const capabilities = provider.hook_capabilities || {};
    const events = Array.isArray(profile.events) ? profile.events : [];
    const requiredEvents = Array.isArray(provider.hook_required_events)
      ? provider.hook_required_events
      : [];
    const runtimeSessions = detail.runtime_sessions || [];
    const recentEvents = detail.recent_events || [];
    const recentErrors = detail.recent_errors || [];

    const supported = Object.keys(profile).length > 0 && !!profile.format;
    const actionIds = providerActionIds(hook.status, supported, capabilities);
    const actionButtons = actionIds.map((id) => renderHookActionButton(providerId, id)).join("");

    return `
      <header class="manager-section-head agent-provider-detail-header">
        <div class="stack agent-provider-detail-head">
          <strong>${escapeHtml(providerDisplayName(provider))}</strong>
          <small>${t("hookProviderDetailHint")}</small>
          <div class="pill-row">
            <span class="pill">${escapeHtml(providerId)}</span>
            <span class="pill">${escapeHtml(hook.status || "unknown")}</span>
            ${profile.format ? `<span class="pill">${escapeHtml(formatHookFormat(profile.format))}</span>` : ""}
          </div>
        </div>
        ${actionButtons ? `<div class="pill-row">${actionButtons}</div>` : ""}
      </header>

      <div class="agent-provider-detail-scroll">
        <div class="manager-summary-grid agent-environment-grid">
          ${renderMetaLine(t("provider"), providerId)}
          ${renderMetaLine("Hook status", hook.status || "unknown")}
          ${renderMetaLine(t("hookRequiredEvents"), String(requiredEvents.length || events.length))}
          ${renderMetaLine("Hook format", formatHookFormat(profile.format))}
          ${renderMetaLine("Last event", hook.last_event_at ? formatDate(hook.last_event_at) : "—")}
          ${renderMetaLine(
            t("hookActiveRuntime"),
            String(runtimeSessions.filter((session) => !["completed", "failed"].includes(session.status)).length)
          )}
          ${renderMetaLine(t("hookLinkedSessions"), String(diagnosis.linked || 0))}
          ${renderMetaLine(t("hookWeakSessions"), String(diagnosis.weakly_linked || 0))}
          ${renderMetaLine(t("hookNoMatchSessions"), String(diagnosis.no_session_match || 0))}
        </div>

        ${renderWideMetaLine("Hook config", hook.config_path || profile.config_hint || "—")}

        ${hook.message ? `<div class="empty-state">${escapeHtml(hook.message)}</div>` : ""}
        ${renderProviderEventProfile(events, requiredEvents)}
        ${renderProviderRuntimeSessions(runtimeSessions)}
        ${renderProviderRecentEvents(recentEvents)}
        ${renderProviderRecentErrors(recentErrors)}
        ${renderProviderSessionDiagnosis(providerId)}
      </div>`;
  }

  function renderProviderEventProfile(events, requiredEvents = []) {
    const eventNames = events.map((event) => event.name);
    const missingRequired = requiredEvents.filter((event) => !eventNames.includes(event));
    const body = events.length
      ? `<div class="settings-list agent-environment-paths">
          <div class="settings-row agent-detail-row">
            <div class="settings-copy settings-copy-inline">
              <strong>${t("hookEventProfile")}</strong>
              <span>${t("hookEventProfileHint")}</span>
            </div>
            <div class="pill-row hook-event-pill-row">
              ${events
                .map(
                  (event) =>
                    `<span class="pill" title="${escapeAttr(event.blocking ? "blocking" : "record-only")}">${escapeHtml(
                      event.name
                    )}${event.blocking ? " *" : ""}</span>`
                )
                .join("")}
            </div>
          </div>
          ${
            missingRequired.length
              ? `<div class="settings-row agent-detail-row">
                  <div class="settings-copy settings-copy-inline">
                    <strong>${t("hookMissingRequiredEvents")}</strong>
                  </div>
                  <div class="pill-row hook-event-pill-row">
                    ${missingRequired
                      .map((name) => `<span class="pill">${escapeHtml(name)}</span>`)
                      .join("")}
                  </div>
                </div>`
              : ""
          }
        </div>`
      : `<div class="empty-state">${t("hookNoRecentEvents")}</div>`;
    return renderDetailBlock(t("hookEventProfile"), t("hookEventProfileHint"), body);
  }

  function renderProviderRuntimeSessions(sessions) {
    const body = sessions.length
      ? `<div class="settings-list agent-environment-paths">${sessions.map(renderRuntimeRow).join("")}</div>`
      : `<div class="empty-state">${t("hookNoActiveRuntime")}</div>`;
    return renderDetailBlock(t("hookRuntimeSessions"), t("hookRuntimeSessionsHint"), body);
  }

  function renderProviderRecentEvents(events) {
    const body = events.length
      ? `<div class="settings-list agent-environment-paths">${events.map(renderEventRow).join("")}</div>`
      : `<div class="empty-state">${t("hookNoRecentEvents")}</div>`;
    return renderDetailBlock(t("hookRecentEvents"), t("hookRecentEventsHint"), body);
  }

  function renderEventRow(event) {
    const subject =
      event.tool?.name || event.message?.role || event.provider_session_id || event.run_id || event.event_id;
    return `
      <div class="settings-row agent-detail-row">
        <div class="settings-copy settings-copy-inline">
          <strong>${escapeHtml(event.event_type || "unknown")}</strong>
          <span>${escapeHtml(String(subject || "—"))}</span>
        </div>
        <div class="pill-row hook-event-pill-row">
          <span class="pill">${escapeHtml(formatDate(event.timestamp))}</span>
          ${event.provider_session_id ? `<span class="pill">${escapeHtml(event.provider_session_id)}</span>` : ""}
        </div>
      </div>`;
  }

  function renderProviderRecentErrors(errors) {
    const body = errors.length
      ? `<div class="settings-list agent-environment-paths">${errors.map(renderErrorRow).join("")}</div>`
      : `<div class="empty-state">${t("hookNoRecentErrors")}</div>`;
    return renderDetailBlock(t("hookRecentErrors"), t("hookRecentErrorsHint"), body);
  }

  function renderProviderSessionDiagnosis(providerId) {
    const groups = state.hooks.sessionDiagnosis || [];
    const rows = [];
    for (const group of groups) {
      if ((group.provider_id || "") !== providerId) continue;
      for (const session of group.sessions || []) {
        rows.push({ group, session });
      }
    }
    if (!rows.length) return "";
    const body = `
      <div class="settings-list agent-environment-paths">
        ${rows.map(({ group, session }) => renderSessionDiagnosisRow(group, session)).join("")}
      </div>`;
    return renderDetailBlock(t("hookSessionDiagnosis"), t("hookSessionDiagnosisHint"), body);
  }

  function providerActionIds(status, supported, capabilities = {}) {
    if (!supported) return [];
    const available = {
      install_hook: capabilities.install !== false,
      verify_hook: capabilities.verify !== false,
      repair_hook: capabilities.repair !== false,
      uninstall_hook: capabilities.uninstall !== false,
    };
    const filterAvailable = (ids) => ids.filter((id) => available[id]);
    if (status === "not_installed") return filterAvailable(["install_hook", "verify_hook"]);
    if (
      [
        "installed_disabled",
        "installed_stale_binary",
        "installed_stale_endpoint",
        "installed_broken_config",
        "installed_conflict",
        "repairable",
        "needs_user_action",
      ].includes(status)
    ) {
      return filterAvailable(["repair_hook", "verify_hook", "uninstall_hook"]);
    }
    if (status === "installed_ok") return filterAvailable(["verify_hook", "repair_hook", "uninstall_hook"]);
    return filterAvailable(["verify_hook"]);
  }

  function renderHookActionButton(providerId, settingId) {
    const key = `${providerId}:${settingId}`;
    const pending = !!state.agents.pendingSettings[key];
    const danger = settingId === "uninstall_hook" ? " danger" : "";
    const action = settingId === "verify_hook" ? "run-hook-operation-now" : "open-hook-operation-confirm";
    return `<button
      type="button"
      class="${settingId === "install_hook" ? "invert" : ""}${danger}"
      data-action="${action}"
      data-provider="${escapeAttr(providerId)}"
      data-setting-id="${escapeAttr(settingId)}"
      ${pending ? "disabled" : ""}
    >${escapeHtml(pending ? t("running") : hookActionLabel(settingId))}</button>`;
  }

  function hookActionLabel(settingId) {
    switch (settingId) {
      case "install_hook":
        return t("hookInstall");
      case "verify_hook":
        return t("hookVerify");
      case "repair_hook":
        return t("hookRepair");
      case "uninstall_hook":
        return t("hookUninstall");
      default:
        return settingId;
    }
  }

  function renderRuntimeRow(session) {
    const workspace = session.cwd || session.correlation?.project_dir || "—";
    const sessionId = session.provider_session_id || session.correlation?.session_id || session.runtime_id || "—";
    const currentTool = session.current_tool?.name || "—";
    return `
      <div class="settings-row agent-detail-row">
        <div class="settings-copy settings-copy-inline">
          <strong>${escapeHtml(session.provider || "unknown")} / ${escapeHtml(String(sessionId))}</strong>
          <span>${escapeHtml(String(workspace))}</span>
        </div>
        <div class="pill-row hook-event-pill-row">
          <span class="pill">${escapeHtml(session.status || "unknown")}</span>
          <span class="pill">${t("currentTool")}=${escapeHtml(currentTool)}</span>
          <span class="pill">${t("updatedAt")}=${escapeHtml(formatDate(session.updated_at || session.last_event_at))}</span>
        </div>
      </div>`;
  }

  function renderSessionDiagnosisRow(group, session) {
    const diagnosis = session.hook_diagnosis || {};
    const summary = session.hook_runtime_summary || {};
    const providerId = session.provider_id || group.provider_id;
    const title = session.display_title || session.title || session.session_id;
    const actionButtons = (diagnosis.actions || []).map((action) => renderDiagnosisAction(providerId, action)).join("");
    return `
      <div class="settings-row agent-detail-row">
        <div class="settings-copy settings-copy-inline">
          <a
            class="session-title session-title-link"
            href="/sessions/${encodeURIComponent(providerId)}/${encodeURIComponent(session.session_id)}"
            data-nav="/sessions/${encodeURIComponent(providerId)}/${encodeURIComponent(session.session_id)}"
          >${escapeHtml(title)}</a>
          <span>${escapeHtml(session.project_dir || "—")}</span>
          <small>${escapeHtml(diagnosis.message || "")}</small>
        </div>
        <div class="hook-provider-main">
          <div class="pill-row hook-event-pill-row">
            <span class="pill">${escapeHtml(providerDisplayNameById(group.provider_id || providerId))}</span>
            <span class="pill">${escapeHtml(diagnosis.kind || "unknown")}</span>
            ${diagnosis.confidence ? `<span class="pill">confidence=${escapeHtml(diagnosis.confidence)}</span>` : ""}
            ${diagnosis.matched_by || summary.matched_by ? `<span class="pill">matched=${escapeHtml(diagnosis.matched_by || summary.matched_by)}</span>` : ""}
          </div>
          ${actionButtons ? `<div class="pill-row">${actionButtons}</div>` : ""}
        </div>
      </div>`;
  }

  function providerDisplayNameById(providerId) {
    return providers.displayName(providerId);
  }

  function renderDiagnosisAction(providerId, action) {
    const settingId = action.setting_id || "";
    if (!settingId) return "";
    const key = `${providerId}:${settingId}`;
    const pending = !!state.agents.pendingSettings[key];
    const dataAction = settingId === "verify_hook" ? "run-hook-operation-now" : "open-hook-operation-confirm";
    return `<button
      type="button"
      class="${settingId === "repair_hook" ? "invert" : ""}"
      data-action="${escapeAttr(dataAction)}"
      data-provider="${escapeAttr(providerId)}"
      data-setting-id="${escapeAttr(settingId)}"
      title="${escapeAttr(action.reason || "")}"
      ${pending ? "disabled" : ""}
    >${escapeHtml(pending ? t("running") : action.label)}</button>`;
  }

  function renderErrorRow(error) {
    return `
      <div class="settings-row agent-detail-row">
        <div class="settings-copy settings-copy-inline">
          <strong>${escapeHtml(error.scope || "hook")}</strong>
          <span>${escapeHtml(formatDate(error.timestamp))}</span>
        </div>
        <div class="path-line">${escapeHtml(formatValue(error.message || "—"))}</div>
      </div>`;
  }

  function renderDetailBlock(title, hint, body) {
    return `
      <div class="stack hook-provider-detail-block">
        <div class="section-heading">
          <div>
            <strong>${escapeHtml(title)}</strong>
            <small>${escapeHtml(hint)}</small>
          </div>
        </div>
        ${body}
      </div>`;
  }

  function formatHookFormat(format) {
    if (!format) return "—";
    return String(format)
      .split("_")
      .map((part) => (part ? part[0].toUpperCase() + part.slice(1) : part))
      .join(" ");
  }

  function renderWideMetaLine(label, value) {
    if (value === null || value === undefined || value === "") return "";
    return `<div class="stack meta-line-wide hook-wide-meta"><span class="eyebrow">${escapeHtml(label)}</span><div class="path-line">${escapeHtml(
      formatValue(value)
    )}</div></div>`;
  }

  return {
    renderHooksCenterPage,
  };
}