export function createHooksCenterModule({
  state,
  providers,
  t,
  escapeHtml,
  escapeAttr,
  formatDate,
  formatValue,
  renderMetaLine,
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
    const summary = overview.summary || {};
    const server = overview.server || {};
    return `
      <section class="manager-workspace-summary">
        <div>
          <span class="eyebrow">${t("hooks")}</span>
          <strong>${t("hooksTitle")}</strong>
          <p>${t("hooksHint")}</p>
        </div>
        <div class="manager-summary-grid agent-environment-grid">
          ${renderMetaLine(t("hookServer"), server.running ? t("running") : t("notDetected"))}
          ${renderMetaLine(t("providers"), String(summary.providers || 0))}
          ${renderMetaLine(t("hookInstalledOk"), String(summary.installed_ok || 0))}
          ${renderMetaLine(t("hookNeedsAttention"), String(summary.needs_attention || 0))}
          ${renderMetaLine(t("hookActiveRuntime"), String(summary.active_runtime_sessions || 0))}
          ${renderMetaLine(t("errors"), String(summary.recent_errors || 0))}
        </div>
        <div class="manager-actions">
          <button type="button" data-action="refresh-hooks">${t("refresh")}</button>
          <button type="button" data-action="run-hook-doctor" data-repair="false">${t("hookDoctor")}</button>
          <button type="button" data-action="cleanup-hook-runtime">${t("hookCleanup")}</button>
        </div>
      </section>`;
  }

  function renderHooksProviderList(providerList) {
    if (!providerList.length) {
      return `<div class="empty-state">${t("noProviders")}</div>`;
    }
    return `
      <div class="manager-list agent-provider-list">
        ${providerList.map(renderHooksProviderListItem).join("")}
      </div>`;
  }

  function renderHooksProviderListItem(provider) {
    const providerId = provider.provider_id || "";
    const selected = state.hooks.selectedProvider === providerId;
    const hook = provider.hook || {};
    const status = hook.status || "unknown";
    const statusClass = status === "installed_ok" ? "is-installed" : "is-missing";
    return `
      <button
        type="button"
        class="agent-provider-item ${selected ? "is-active" : ""}"
        data-action="select-hook-provider"
        data-provider="${escapeAttr(providerId)}"
      >
        <span class="agent-provider-head">
          <strong class="agent-provider-name">${escapeHtml(providerDisplayName(provider))}</strong>
          <span class="agent-provider-state ${statusClass}" title="${escapeAttr(status)}" aria-label="${escapeAttr(status)}">
            ${status === "installed_ok" ? "●" : "○"}
          </span>
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
          ${renderMetaLine(t("hookCapabilities"), formatCapabilities(capabilities))}
          ${renderMetaLine(t("hookRequiredEvents"), String(requiredEvents.length || events.length))}
          ${renderMetaLine("Hook format", formatHookFormat(profile.format))}
          ${renderMetaLine("Hook config", hook.config_path || profile.config_hint || "—")}
          ${renderMetaLine("Installed version", hook.installed_version || "—")}
          ${renderMetaLine("Current version", hook.current_version || "—")}
          ${renderMetaLine("Last event", hook.last_event_at ? formatDate(hook.last_event_at) : "—")}
          ${renderMetaLine(
            t("hookActiveRuntime"),
            String(runtimeSessions.filter((session) => !["completed", "failed"].includes(session.status)).length)
          )}
          ${renderMetaLine(t("hookLinkedSessions"), String(diagnosis.linked || 0))}
          ${renderMetaLine(t("hookWeakSessions"), String(diagnosis.weakly_linked || 0))}
          ${renderMetaLine(t("hookNoMatchSessions"), String(diagnosis.no_session_match || 0))}
        </div>

        ${hook.message ? `<div class="empty-state">${escapeHtml(hook.message)}</div>` : ""}
        ${renderProviderEventProfile(events, requiredEvents)}
        ${renderProviderRuntimeSessions(runtimeSessions)}
        ${renderProviderRecentEvents(recentEvents)}
        ${renderProviderRecentErrors(recentErrors)}
        ${renderProviderRecommendedActions(providerId, diagnosis.recommended_actions || [])}
        ${renderProviderSessionDiagnosis(providerId)}
      </div>`;
  }

  function renderProviderEventProfile(events, requiredEvents = []) {
    const eventNames = events.map((event) => event.name);
    const missingRequired = requiredEvents.filter((event) => !eventNames.includes(event));
    return `
      <div class="stack hook-provider-detail-block">
        <div class="section-heading">
          <div>
            <strong>${t("hookEventProfile")}</strong>
            <small>${t("hookEventProfileHint")} · ${t("hookRequiredEvents")}=${escapeHtml(
              String(requiredEvents.length || events.length)
            )}</small>
          </div>
        </div>
        <div class="pill-row hook-event-pill-row">
          ${
            events.length
              ? events
                  .map(
                    (event) =>
                      `<span class="pill" title="${escapeAttr(event.blocking ? "blocking" : "record-only")}">${escapeHtml(
                        event.name
                      )}${event.blocking ? " *" : ""}</span>`
                  )
                  .join("")
              : `<span class="pill">—</span>`
          }
        </div>
        ${
          missingRequired.length
            ? `<div class="empty-state">${escapeHtml(t("hookMissingRequiredEvents"))}: ${escapeHtml(
                missingRequired.join(", ")
              )}</div>`
            : ""
        }
      </div>`;
  }

  function renderProviderRuntimeSessions(sessions) {
    return `
      <div class="stack hook-provider-detail-block">
        <div class="section-heading">
          <div>
            <strong>${t("hookRuntimeSessions")}</strong>
            <small>${t("hookRuntimeSessionsHint")}</small>
          </div>
        </div>
        <div class="settings-list">
          ${sessions.length ? sessions.map(renderRuntimeRow).join("") : `<div class="empty-state">${t("hookNoActiveRuntime")}</div>`}
        </div>
      </div>`;
  }

  function renderProviderRecentEvents(events) {
    return `
      <div class="stack hook-provider-detail-block">
        <div class="section-heading">
          <div>
            <strong>${t("hookRecentEvents")}</strong>
            <small>${t("hookRecentEventsHint")}</small>
          </div>
        </div>
        <div class="settings-list">
          ${events.length ? events.map(renderEventRow).join("") : `<div class="empty-state">${t("hookNoRecentEvents")}</div>`}
        </div>
      </div>`;
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
    return `
      <div class="stack hook-provider-detail-block">
        <div class="section-heading">
          <div>
            <strong>${t("hookRecentErrors")}</strong>
            <small>${t("hookRecentErrorsHint")}</small>
          </div>
        </div>
        ${
          errors.length
            ? `<div class="settings-list">${errors.map(renderErrorRow).join("")}</div>`
            : `<div class="empty-state">${t("hookNoRecentErrors")}</div>`
        }
      </div>`;
  }

  function renderProviderRecommendedActions(providerId, actions) {
    if (!actions.length) return "";
    return `
      <div class="stack hook-provider-detail-block">
        <div class="section-heading">
          <div>
            <strong>${t("hookRecommendedActions")}</strong>
            <small>${t("hookRecommendedActionsHint")}</small>
          </div>
        </div>
        <div class="settings-list">
          ${actions
            .map(
              (action) => `
                <div class="settings-row agent-detail-row">
                  <div class="settings-copy settings-copy-inline">
                    <strong>${escapeHtml(action.label)}</strong>
                    <span>${escapeHtml(action.reason)}</span>
                  </div>
                  <div class="pill-row">
                    ${renderDiagnosisAction(providerId, action)}
                  </div>
                </div>`
            )
            .join("")}
        </div>
      </div>`;
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
    return `
      <div class="stack hook-provider-detail-block">
        <div class="section-heading">
          <div>
            <strong>${t("hookSessionDiagnosis")}</strong>
            <small>${t("hookSessionDiagnosisHint")}</small>
          </div>
        </div>
        <div class="settings-list">
          ${rows.map(({ group, session }) => renderSessionDiagnosisRow(group, session)).join("")}
        </div>
      </div>`;
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

  function formatHookFormat(format) {
    if (!format) return "—";
    return String(format)
      .split("_")
      .map((part) => (part ? part[0].toUpperCase() + part.slice(1) : part))
      .join(" ");
  }

  function formatCapabilities(capabilities) {
    const enabled = ["detect", "verify", "install", "repair", "uninstall"].filter(
      (key) => capabilities[key] === true
    );
    return enabled.length ? enabled.join(" / ") : "—";
  }

  return {
    renderHooksCenterPage,
  };
}