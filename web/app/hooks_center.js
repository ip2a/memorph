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
      <section class="manager-hero">
        <div>
          <p class="eyebrow">${t("hooks")}</p>
          <h1>${t("hooksTitle")}</h1>
          <p class="muted">${t("hooksHint")}</p>
        </div>
        <div class="manager-actions">
          <button type="button" data-action="refresh-hooks">${t("refresh")}</button>
          <button type="button" data-action="run-hook-doctor" data-repair="false">${t("hookDoctor")}</button>
          <button type="button" data-action="cleanup-hook-runtime">${t("hookCleanup")}</button>
        </div>
      </section>
      ${renderOverviewSummary(overview)}
      ${renderProviderMatrix(overview.providers || [])}
      ${renderProviderDetail()}
      ${renderRuntimeSessions(overview.runtime_sessions || [])}
      ${renderDiagnosisPanel(state.hooks.sessionDiagnosis || [], overview.providers || [])}
      ${renderRecentErrors(overview.recent_errors || [])}
    `;
  }

  function renderOverviewSummary(overview) {
    const summary = overview.summary || {};
    const server = overview.server || {};
    return `
      <section class="section-panel">
        <div class="section-heading">
          <div>
            <strong>${t("hookOverview")}</strong>
            <small>${t("hookOverviewHint")}</small>
          </div>
        </div>
        <div class="manager-summary-grid agent-environment-grid">
          ${renderMetaLine(t("hookServer"), server.running ? t("running") : t("notDetected"))}
          ${renderMetaLine(t("providers"), String(summary.providers || 0))}
          ${renderMetaLine(t("hookSupportedProviders"), String(summary.supported_providers || 0))}
          ${renderMetaLine(t("hookInstalledOk"), String(summary.installed_ok || 0))}
          ${renderMetaLine(t("hookNotInstalled"), String(summary.not_installed || 0))}
          ${renderMetaLine(t("hookNeedsAttention"), String(summary.needs_attention || 0))}
          ${renderMetaLine(t("hookActiveRuntime"), String(summary.active_runtime_sessions || 0))}
          ${renderMetaLine(t("hookLinkedSessions"), String(summary.linked_sessions || 0))}
          ${renderMetaLine(t("hookWeakSessions"), String(summary.weakly_linked_sessions || 0))}
          ${renderMetaLine(t("hookNoMatchSessions"), String(summary.no_session_match || 0))}
          ${renderMetaLine(t("hookObservedBlocking"), String(summary.observed_blocking_requests || 0))}
          ${renderMetaLine(t("errors"), String(summary.recent_errors || 0))}
        </div>
        <div class="empty-state">${escapeHtml(t("hookRecordOnlyBoundary"))}</div>
      </section>`;
  }

  function renderProviderMatrix(providers) {
    return `
      <section class="section-panel">
        <div class="section-heading">
          <div>
            <strong>${t("hookProviderMatrix")}</strong>
            <small>${t("hookProviderMatrixHint")}</small>
          </div>
        </div>
        <div class="settings-list">
          ${providers.length ? providers.map(renderProviderRow).join("") : `<div class="empty-state">${t("noProviders")}</div>`}
        </div>
      </section>`;
  }

  function renderProviderRow(provider) {
    const hook = provider.hook || {};
    const diagnosis = provider.hook_diagnosis || {};
    const profile = provider.hook_profile || {};
    const capabilities = provider.hook_capabilities || {};
    const requiredEvents = Array.isArray(provider.hook_required_events) ? provider.hook_required_events : [];
    const providerId = provider.provider_id || hook.provider || "";
    const supported = !!provider.hook_profile;
    const actionIds = providerActionIds(hook.status, supported, capabilities);
    const selected = state.hooks.selectedProvider === providerId;
    return `
      <div class="settings-row agent-detail-row hook-provider-row ${selected ? "is-active" : ""}">
        <div class="settings-copy settings-copy-inline">
          <button
            type="button"
            class="meta-link-button"
            data-action="select-hook-provider"
            data-provider="${escapeAttr(providerId)}"
          >${escapeHtml(providers.displayName(providerId))}</button>
          <span>${escapeHtml(hook.message || profile.config_hint || t("hookOptionalInstallHint"))}</span>
        </div>
        <div class="hook-provider-main">
          <div class="pill-row hook-event-pill-row">
            <span class="pill">${escapeHtml(hook.status || "unknown")}</span>
            <span class="pill">${escapeHtml(formatHookFormat(profile.format))}</span>
            <span class="pill">${t("hookCapabilities")}=${escapeHtml(formatCapabilities(capabilities))}</span>
            <span class="pill">${t("hookRequiredEvents")}=${escapeHtml(String(requiredEvents.length || (Array.isArray(profile.events) ? profile.events.length : 0)))}</span>
            <span class="pill">${t("hookLinkedSessions")}=${escapeHtml(String(diagnosis.linked || 0))}</span>
            <span class="pill">${t("hookWeakSessions")}=${escapeHtml(String(diagnosis.weakly_linked || 0))}</span>
            <span class="pill">${t("hookActiveRuntime")}=${escapeHtml(String(diagnosis.active_runtime_sessions || 0))}</span>
          </div>
          <div class="pill-row">
            ${actionIds.map((settingId) => renderHookActionButton(providerId, settingId)).join("")}
          </div>
        </div>
      </div>`;
  }

  function renderProviderDetail() {
    const detail = state.hooks.providerDetail;
    const provider = detail?.provider;
    const providerId = provider?.provider_id || "";
    if (!provider) {
      return `
        <section class="section-panel">
          <div class="empty-state">${t("hookSelectProvider")}</div>
        </section>`;
    }
    const hook = provider.hook || {};
    const diagnosis = provider.hook_diagnosis || {};
    const profile = provider.hook_profile || {};
    const capabilities = provider.hook_capabilities || {};
    const events = Array.isArray(profile.events) ? profile.events : [];
    const requiredEvents = Array.isArray(provider.hook_required_events) ? provider.hook_required_events : [];
    const runtimeSessions = detail.runtime_sessions || [];
    const recentEvents = detail.recent_events || [];
    const recentErrors = detail.recent_errors || [];
    return `
      <section class="section-panel">
        <div class="section-heading">
          <div>
            <strong>${escapeHtml(providers.displayName(providerId))} ${t("hookProviderDetail")}</strong>
            <small>${t("hookProviderDetailHint")}</small>
          </div>
        </div>
        <div class="manager-summary-grid agent-environment-grid">
          ${renderMetaLine(t("provider"), provider.provider_id)}
          ${renderMetaLine("Hook status", hook.status || "unknown")}
          ${renderMetaLine(t("hookCapabilities"), formatCapabilities(capabilities))}
          ${renderMetaLine(t("hookRequiredEvents"), String(requiredEvents.length || events.length))}
          ${renderMetaLine("Hook format", formatHookFormat(profile.format))}
          ${renderMetaLine("Hook config", hook.config_path || profile.config_hint || "—")}
          ${renderMetaLine("Installed version", hook.installed_version || "—")}
          ${renderMetaLine("Current version", hook.current_version || "—")}
          ${renderMetaLine("Last event", hook.last_event_at ? formatDate(hook.last_event_at) : "—")}
          ${renderMetaLine(t("hookActiveRuntime"), String(runtimeSessions.filter((session) => !["completed", "failed"].includes(session.status)).length))}
          ${renderMetaLine(t("hookLinkedSessions"), String(diagnosis.linked || 0))}
          ${renderMetaLine(t("hookWeakSessions"), String(diagnosis.weakly_linked || 0))}
          ${renderMetaLine(t("hookNoMatchSessions"), String(diagnosis.no_session_match || 0))}
        </div>
        ${hook.message ? `<div class="empty-state">${escapeHtml(hook.message)}</div>` : ""}
        ${renderProviderEventProfile(events, requiredEvents)}
        ${renderProviderRuntimeSessions(runtimeSessions)}
        ${renderProviderRecentEvents(recentEvents)}
        ${renderProviderRecentErrors(recentErrors)}
      </section>`;
  }

  function renderProviderEventProfile(events, requiredEvents = []) {
    const eventNames = events.map((event) => event.name);
    const missingRequired = requiredEvents.filter((event) => !eventNames.includes(event));
    return `
      <div class="stack hook-provider-detail-block">
        <div class="section-heading">
          <div>
            <strong>${t("hookEventProfile")}</strong>
            <small>${t("hookEventProfileHint")} · ${t("hookRequiredEvents")}=${escapeHtml(String(requiredEvents.length || events.length))}</small>
          </div>
        </div>
        <div class="pill-row hook-event-pill-row">
          ${
            events.length
              ? events.map((event) => `<span class="pill" title="${escapeAttr(event.blocking ? "blocking" : "record-only")}">${escapeHtml(event.name)}${event.blocking ? " *" : ""}</span>`).join("")
              : `<span class="pill">—</span>`
          }
        </div>
        ${
          missingRequired.length
            ? `<div class="empty-state">${escapeHtml(t("hookMissingRequiredEvents"))}: ${escapeHtml(missingRequired.join(", "))}</div>`
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
    const subject = event.tool?.name || event.message?.role || event.provider_session_id || event.run_id || event.event_id;
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
    if (["installed_disabled", "installed_stale_binary", "installed_stale_endpoint", "installed_broken_config", "installed_conflict", "repairable", "needs_user_action"].includes(status)) {
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

  function renderRuntimeSessions(sessions) {
    const active = sessions.filter((session) => !["completed", "failed"].includes(session.status));
    return `
      <section class="section-panel">
        <div class="section-heading">
          <div>
            <strong>${t("hookRuntimeSessions")}</strong>
            <small>${t("hookRuntimeSessionsHint")}</small>
          </div>
        </div>
        <div class="settings-list">
          ${active.length ? active.map(renderRuntimeRow).join("") : `<div class="empty-state">${t("hookNoActiveRuntime")}</div>`}
        </div>
      </section>`;
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

  function renderDiagnosisPanel(groups, providers) {
    const filters = hookDiagnosisFilters();
    const activeFilter = state.hooks.diagnosisFilter || "attention";
    const sessionRows = groups.flatMap((group) =>
      (group.sessions || []).map((session) => ({
        group,
        session,
      }))
    );
    const providerRows = providers.filter((provider) => {
      const diagnosis = provider.hook_diagnosis || {};
      return (
        (diagnosis.hook_needs_attention || 0) +
          (diagnosis.no_session_match || 0) +
          (diagnosis.no_active_runtime || 0) +
          (diagnosis.no_events_yet || 0) +
          (diagnosis.weakly_linked || 0) >
        0
      );
    });
    return `
      <section class="section-panel">
        <div class="section-heading">
          <div>
            <strong>${t("hookSessionDiagnosis")}</strong>
            <small>${t("hookSessionDiagnosisHint")} · ${escapeHtml(hookDiagnosisFilterLabel(activeFilter))}</small>
          </div>
          <div class="pill-row hook-event-pill-row">
            ${filters
              .map(
                (filter) => `<button
                  type="button"
                  class="${filter.id === activeFilter ? "invert" : ""}"
                  data-action="set-hook-diagnosis-filter"
                  data-filter="${escapeAttr(filter.id)}"
                >${escapeHtml(filter.label)}</button>`
              )
              .join("")}
          </div>
        </div>
        <div class="settings-list">
          ${
            sessionRows.length
              ? sessionRows.map(({ group, session }) => renderSessionDiagnosisRow(group, session)).join("")
              : providerRows.length
                ? providerRows.map(renderDiagnosisRow).join("")
                : `<div class="empty-state">${t("hookNoDiagnosisIssues")}</div>`
          }
        </div>
      </section>`;
  }

  function hookDiagnosisFilters() {
    return [
      { id: "attention", label: t("hookFilterAttention") },
      { id: "weak", label: t("hookFilterWeak") },
      { id: "no_match", label: t("hookFilterNoMatch") },
      { id: "runtime", label: t("hookFilterRuntime") },
      { id: "linked", label: t("hookFilterLinked") },
      { id: "no_hook", label: t("hookFilterNoHook") },
    ];
  }

  function hookDiagnosisFilterLabel(filterId) {
    return hookDiagnosisFilters().find((filter) => filter.id === filterId)?.label || filterId;
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
            <span class="pill">${escapeHtml(providers.displayName(group.provider_id || providerId))}</span>
            <span class="pill">${escapeHtml(diagnosis.kind || "unknown")}</span>
            ${diagnosis.confidence ? `<span class="pill">confidence=${escapeHtml(diagnosis.confidence)}</span>` : ""}
            ${diagnosis.matched_by || summary.matched_by ? `<span class="pill">matched=${escapeHtml(diagnosis.matched_by || summary.matched_by)}</span>` : ""}
          </div>
          ${actionButtons ? `<div class="pill-row">${actionButtons}</div>` : ""}
        </div>
      </div>`;
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
    >${escapeHtml(pending ? t("running") : hookActionLabel(settingId))}</button>`;
  }

  function renderDiagnosisRow(provider) {
    const diagnosis = provider.hook_diagnosis || {};
    const providerId = provider.provider_id || "";
    return `
      <div class="settings-row agent-detail-row">
        <div class="settings-copy settings-copy-inline">
          <strong>${escapeHtml(providers.displayName(providerId))}</strong>
          <span>${escapeHtml(t("hookDiagnosisProviderHint"))}</span>
        </div>
        <div class="pill-row hook-event-pill-row">
          <span class="pill">needs=${escapeHtml(String(diagnosis.hook_needs_attention || 0))}</span>
          <span class="pill">weak=${escapeHtml(String(diagnosis.weakly_linked || 0))}</span>
          <span class="pill">no_match=${escapeHtml(String(diagnosis.no_session_match || 0))}</span>
          <span class="pill">no_active=${escapeHtml(String(diagnosis.no_active_runtime || 0))}</span>
          <span class="pill">no_events=${escapeHtml(String(diagnosis.no_events_yet || 0))}</span>
        </div>
      </div>`;
  }

  function renderRecentErrors(errors) {
    return `
      <section class="section-panel">
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
      </section>`;
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
