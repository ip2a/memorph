export function createAgentsSettingsModule({
  state,
  providers,
  aboutLinks,
  t,
  escapeHtml,
  escapeAttr,
  formatDate,
  formatValue,
  workspaceName,
  renderMetaLine,
  render,
}) {
  const HOOK_SETTING_IDS = new Set([
    "install_hook",
    "verify_hook",
    "repair_hook",
    "uninstall_hook",
  ]);

  function renderUpdateCheckSummary() {
    if (state.updateCheck.error) {
      return `<span class="settings-update-status is-error">${escapeHtml(`${t("updateCheckFailed")}: ${state.updateCheck.error}`)}</span>`;
    }
    if (!state.updateCheck.result) return "";

    const result = state.updateCheck.result;
    const statusText = result.has_update ? t("updateAvailableStatus") : t("upToDateStatus");
    const summaryText = `${t("installSource")}: ${result.install_source_label} · ${t("latestVersionLabel")}: v${result.latest_version}`;
    const commandText = `${t("updateCommandLabel")}: ${result.update_command}`;

    return `
      <span class="settings-update-status ${result.has_update ? "is-available" : ""}">${escapeHtml(statusText)}</span>
      <span class="settings-update-meta">${escapeHtml(summaryText)}</span>
      <span class="settings-update-command">${escapeHtml(commandText)}</span>`;
  }

  function renderAboutLinks() {
    return aboutLinks
      .map(
        (item) => `
        <a class="settings-about-link" href="${escapeAttr(item.url)}" target="_blank" rel="noopener noreferrer" data-action="open-external" data-url="${escapeAttr(item.url)}">
          <span class="settings-about-link-icon" aria-hidden="true">
            <img src="${escapeAttr(item.iconUrl)}" alt="">
          </span>
          <span class="settings-about-link-copy">
            <strong>${escapeHtml(item.label)}</strong>
            <span>${escapeHtml(item.url)}</span>
          </span>
        </a>`
      )
      .join("");
  }

  function openSettingsModal(draft = null) {
    const settings = draft || state.meta.settings;
    const activeSection = state.ui.settingsSection || "general";
    const settingsSectionClass = (section) => `settings-section ${activeSection === section ? "is-active" : "is-hidden"}`;
    const settingsNavClass = (section) => `settings-nav-item ${activeSection === section ? "is-active" : ""}`;
    const items = providers
      .all()
      .map((provider, index) => {
        const providerId = provider.provider_id;
        const installed = providers.isInstalled(providerId);
        const hidden = providers.isHiddenGlobal(providerId);
        return `
          <div class="settings-provider-row ${hidden ? "is-hidden" : ""}">
            <div class="settings-copy">
              <div class="settings-provider-name">
                <strong>${escapeHtml(providers.displayName(providerId))}</strong>
                <span class="settings-provider-status ${installed ? "is-installed" : "is-missing"}">${installed ? t("installed") : t("notDetected")}</span>
              </div>
              <span>${escapeHtml(providerId)}</span>
              <input type="hidden" name="agent_order" value="${escapeAttr(providerId)}">
            </div>
            <div class="settings-agent-list">
              <label class="settings-check">
                <input type="checkbox" name="hidden_agents" value="${escapeAttr(providerId)}" ${hidden ? "checked" : ""}>
                <span>${t("hidden")}</span>
              </label>
              <button type="button" class="ghost" data-action="shift-agent-up" data-index="${index}">${t("moveUp")}</button>
              <button type="button" class="ghost" data-action="shift-agent-down" data-index="${index}">${t("moveDown")}</button>
            </div>
          </div>`;
      })
      .join("");

    state.modal = {
      kind: "form",
      title: t("settingsTitle"),
      submit: "save-settings",
      className: "settings-modal-card",
      actionsInHead: true,
      body: `
        <div class="settings-layout">
          <nav class="settings-sidebar" aria-label="${escapeAttr(t("settingsTitle"))}">
            <button type="button" class="${settingsNavClass("general")}" data-action="switch-settings-section" data-section="general">${t("general")}</button>
            <button type="button" class="${settingsNavClass("display")}" data-action="switch-settings-section" data-section="display">${t("display")}</button>
            <button type="button" class="${settingsNavClass("order")}" data-action="switch-settings-section" data-section="order">${t("order")}</button>
            <button type="button" class="${settingsNavClass("config")}" data-action="switch-settings-section" data-section="config">${t("configFile")}</button>
            <button type="button" class="${settingsNavClass("about")}" data-action="switch-settings-section" data-section="about">${t("about")}</button>
          </nav>
          <div class="settings-content">
            <section class="${settingsSectionClass("general")}" id="settings-general">
              <div class="settings-section-head">
                <h4>${t("general")}</h4>
              </div>
              <div class="settings-list">
                <div class="settings-row">
                  <div class="settings-copy">
                    <strong>${t("language")}</strong>
                    <span>${t("settingsLanguageHint")}</span>
                  </div>
                  <select name="language">
                    <option value="zh" ${settings.language === "zh" ? "selected" : ""}>中文</option>
                    <option value="en" ${settings.language === "en" ? "selected" : ""}>English</option>
                  </select>
                </div>
                <div class="settings-row">
                  <div class="settings-copy">
                    <strong>${t("defaultBackupDir")}</strong>
                    <span>${t("defaultBackupDirHint")}</span>
                  </div>
                  <div class="path-picker">
                    <input name="default_backup_dir" list="known-workspaces" value="${escapeAttr(
                      settings.default_backup_dir || "./backups"
                    )}" placeholder="./backups">
                    <button type="button" class="ghost" data-action="browse-folder" data-target-field="default_backup_dir">${t(
                      "browse"
                    )}</button>
                  </div>
                </div>
                <div class="settings-row">
                  <div class="settings-copy">
                    <strong>${t("logSettings")}</strong>
                    <span>${t("logSettingsHint")}</span>
                  </div>
                  <div class="settings-agent-list">
                    <label class="field compact-number-field">
                      <span>${t("logMaxSizeMb")}</span>
                      <input type="text" inputmode="decimal" name="log_max_size_mb" value="${escapeAttr(
                        String(((settings.logging?.max_size_bytes || 5 * 1024 * 1024) / 1024 / 1024).toFixed(1).replace(/\.0$/, ""))
                      )}">
                    </label>
                    <label class="field compact-number-field">
                      <span>${t("logRetentionDays")}</span>
                      <input type="text" inputmode="numeric" name="log_retention_days" value="${escapeAttr(
                        settings.logging?.retention_days == null ? "" : String(settings.logging.retention_days)
                      )}" placeholder="${escapeAttr(t("unlimited"))}">
                    </label>
                  </div>
                </div>
              </div>
            </section>
            <section class="${settingsSectionClass("display")}" id="settings-display">
              <div class="settings-section-head">
                <h4>${t("display")}</h4>
              </div>
              <div class="settings-list">
                <div class="settings-row">
                  <div class="settings-copy">
                    <strong>${t("sessionsPerProvider")}</strong>
                    <span>${t("settingsSessionsHint")}</span>
                  </div>
                  <input type="number" min="1" max="200" name="sessions_per_provider" value="${settings.sessions_per_provider}">
                </div>
                <div class="settings-row">
                  <div class="settings-copy">
                    <strong>${t("homeButtons")}</strong>
                    <span>${t("settingsHomeButtonsHint")}</span>
                  </div>
                  <div class="settings-agent-list">
                    <label class="settings-check">
                      <input type="checkbox" name="home_button_view" value="true" ${settings.home_buttons.view ? "checked" : ""}>
                      <span>${t("showView")}</span>
                    </label>
                    <label class="settings-check">
                      <input type="checkbox" name="home_button_switch" value="true" ${settings.home_buttons.switch ? "checked" : ""}>
                      <span>${t("showSwitch")}</span>
                    </label>
                    <label class="settings-check">
                      <input type="checkbox" name="home_button_export" value="true" ${settings.home_buttons.export ? "checked" : ""}>
                      <span>${t("showExport")}</span>
                    </label>
                    <label class="settings-check">
                      <input type="checkbox" name="home_button_sync" value="true" ${settings.home_buttons.sync ? "checked" : ""}>
                      <span>${t("showSync")}</span>
                    </label>
                    <label class="settings-check">
                      <input type="checkbox" name="home_button_delete" value="true" ${settings.home_buttons.delete ? "checked" : ""}>
                      <span>${t("showDelete")}</span>
                    </label>
                  </div>
                </div>
              </div>
            </section>
            <section class="${settingsSectionClass("order")}" id="settings-order">
              <div class="settings-section-head">
                <h4>${t("order")}</h4>
              </div>
              <div class="settings-list">
                <div class="settings-row settings-row-stacked">
                  <div class="settings-copy">
                    <strong>${t("providers")}</strong>
                    <span>${t("settingsProvidersHint")}</span>
                  </div>
                  <div class="settings-provider-list settings-provider-list-vertical">${items}</div>
                </div>
              </div>
            </section>
            <section class="${settingsSectionClass("config")}" id="settings-config">
              <div class="settings-section-head">
                <h4>${t("configFile")}</h4>
              </div>
              <div class="settings-list">
                <div class="settings-row settings-row-stacked">
                  <div class="settings-copy">
                    <strong>${escapeHtml(state.meta?.config_file?.path || "")}</strong>
                    <span>${escapeHtml(state.meta?.config_file?.format || "json")}</span>
                  </div>
                  <pre class="settings-config-content"><code>${escapeHtml(state.meta?.config_file?.content || "")}</code></pre>
                </div>
              </div>
            </section>
            <section class="${settingsSectionClass("about")}" id="settings-about">
              <div class="settings-section-head">
                <h4>${t("about")}</h4>
              </div>
              <div class="settings-list">
                <div class="settings-row settings-row-stacked settings-about-row">
                  <div class="settings-about-head">
                    <div class="settings-copy">
                      <strong>${t("version")}</strong>
                      <span>v${escapeHtml(state.meta?.version || "")}</span>
                    </div>
                    <button type="button" data-action="check-update" ${state.updateCheck.checking ? "disabled" : ""}>
                      ${state.updateCheck.checking ? t("checkingUpdate") : t("checkUpdate")}
                    </button>
                  </div>
                  <div class="settings-about-links">
                    ${renderAboutLinks()}
                  </div>
                  <div class="settings-about-update">
                    ${renderUpdateCheckSummary()}
                  </div>
                </div>
              </div>
            </section>
          </div>
        </div>`,
      submitLabel: t("save"),
    };
    render();
  }

  function renderAgentManagementPage() {
    const selected = currentAgentProvider();
    return `
      <div class="manager-page-layout agent-management-page-layout">
        <section class="section-panel manager-control-panel agent-provider-panel">
          <section class="manager-workspace-summary">
            <div>
              <span class="eyebrow">${t("workspace")}</span>
              <strong>${escapeHtml(workspaceName(state.home.workspace) || t("workspaceEmpty"))}</strong>
              <p>${escapeHtml(state.home.workspace || "—")}</p>
            </div>
          </section>
          ${renderAgentProviderList()}
        </section>
        <section class="section-panel manager-result-panel agent-provider-detail-panel">
          ${selected ? renderAgentProviderDetail(selected) : `<div class="empty-state">${t("noProviders")}</div>`}
        </section>
      </div>`;
  }

  function renderHookDiagnosticsPanel() {
    const report = state.agents.hookDiagnostics;
    return `
      <section class="section-panel manager-result-panel agent-provider-detail-panel">
        <div class="section-heading">
          <div>
            <strong>Hook Diagnostics</strong>
            <small>Read-only support snapshot for hook runtime, provider installs, events, and errors.</small>
          </div>
          <div class="pill-row">
            <button type="button" class="invert" data-action="load-hook-diagnostics">Refresh diagnostics</button>
            <button type="button" class="invert" data-action="run-hook-doctor" data-repair="false">Doctor check</button>
            <button type="button" data-action="run-hook-doctor" data-repair="true">Doctor repair</button>
            <button type="button" class="invert" data-action="cleanup-hook-runtime">Cleanup runtime</button>
          </div>
        </div>
        ${
          report
            ? renderHookDiagnosticsReport(report)
            : `<div class="empty-state">No diagnostics loaded yet.</div>`
        }
      </section>`;
  }

  function renderHookDiagnosticsReport(report) {
    const counts = report.counts || {};
    const server = report.server || {};
    const providers = report.providers || [];
    const errors = report.recent_errors || [];
    return `
      <div class="stack">
        <div class="manager-summary-grid agent-environment-grid">
          ${renderMetaLine("Generated", report.generated_at ? formatDate(report.generated_at) : "—")}
          ${renderMetaLine("Server", server.endpoint || "not running")}
          ${renderMetaLine("Runtime sessions", String(counts.runtime_sessions || 0))}
          ${renderMetaLine("Active runtime sessions", String(counts.active_runtime_sessions || 0))}
          ${renderMetaLine("Recent events", String(counts.recent_events || 0))}
          ${renderMetaLine("Recent errors", String(counts.recent_errors || 0))}
          ${report.store?.root ? renderWideMetaLine("Hook store", report.store.root) : ""}
        </div>
        ${renderHookDoctorReport(state.agents.hookDoctorReport)}
        ${renderHookCleanupReport(state.agents.hookCleanupReport)}
        <div class="settings-list">
          ${providers
            .map((provider) =>
              renderAgentDetailRow(
                `${agentProviderDisplayName({ provider_id: provider.provider })} hook`,
                provider.status || "unknown",
                `${provider.message || provider.config_path || "—"} · capabilities ${formatHookCapabilities(provider.capabilities || {})} · installed ${provider.installed_version || "—"} / current ${
                  provider.current_version || "—"
                }`
              )
            )
            .join("")}
        </div>
        ${
          errors.length
            ? `<pre class="code-block">${escapeHtml(
                JSON.stringify(
                  errors.map((error) => ({
                    timestamp: error.timestamp,
                    scope: error.scope,
                    message: error.message,
                  })),
                  null,
                  2
                )
              )}</pre>`
            : `<div class="empty-state">No recent hook errors.</div>`
        }
      </div>`;
  }

  function renderHookDoctorReport(report) {
    if (!report) return "";
    const results = report.results || [];
    return `
      <div class="stack">
        <div class="manager-summary-grid agent-environment-grid">
          ${renderMetaLine("Doctor checked", String(report.checked || 0))}
          ${renderMetaLine("Doctor repaired", String(report.repaired || 0))}
          ${renderMetaLine("Doctor failed", String(report.failed || 0))}
        </div>
        ${
          results.length
            ? `<div class="settings-list agent-environment-paths">${results
                .map((item) => {
                  const after = item.after?.status ? ` → ${item.after.status}` : "";
                  const error = item.error ? ` · ${item.error}` : "";
                  return renderAgentDetailRow(
                    `${agentProviderDisplayName({ provider_id: item.provider })} doctor`,
                    `${item.before?.status || "unknown"}${after}`,
                    `${item.operation?.message || item.before?.message || "—"}${error}`
                  );
                })
                .join("")}</div>`
            : ""
        }
      </div>`;
  }

  function renderHookCleanupReport(report) {
    if (!report) return "";
    return `
      <div class="manager-summary-grid agent-environment-grid">
        ${renderMetaLine("Cleanup idle", String(report.idle || 0))}
        ${renderMetaLine("Cleanup orphaned", String(report.orphaned || 0))}
      </div>`;
  }

  function currentAgentProvider() {
    return (
      state.agents.providers.find((item) => item.provider_id === state.agents.selectedProvider) ||
      state.agents.providers[0] ||
      null
    );
  }

  function renderAgentProviderList() {
    if (!state.agents.providers.length) {
      return `<div class="empty-state">${t("noProviders")}</div>`;
    }
    return `
      <div class="manager-list agent-provider-list">
        ${state.agents.providers
          .map((item) => {
            const selected = item.provider_id === state.agents.selectedProvider;
            const installed = agentProviderEnvironment(item).installed;
            const statusLabel = installed ? t("installed") : t("notDetected");
            return `
              <button
                type="button"
                class="agent-provider-item ${selected ? "is-active" : ""}"
                data-action="select-agent-provider"
                data-provider="${escapeAttr(item.provider_id)}"
              >
                <span class="agent-provider-head">
                  <strong class="agent-provider-name">${escapeHtml(agentProviderDisplayName(item))}</strong>
                  <span class="agent-provider-state ${installed ? "is-installed" : "is-missing"}" title="${escapeAttr(statusLabel)}" aria-label="${escapeAttr(statusLabel)}">
                    ${installed ? "●" : "○"}
                  </span>
                </span>
              </button>`;
          })
          .join("")}
      </div>`;
  }

  function renderAgentProviderDetail(provider) {
    const settings = provider.settings || [];
    const items = settings.filter(
      (setting) =>
        (setting.kind === "toggle" || setting.kind === "action") &&
        !HOOK_SETTING_IDS.has(setting.id)
    );
    const actionItems = items.filter((setting) => setting.kind === "action");
    const environment = agentProviderEnvironment(provider);
    return `
      <div class="agent-provider-detail-scroll">
      <header class="manager-section-head agent-provider-detail-header">
        <div class="stack agent-provider-detail-head">
          <strong>${escapeHtml(agentProviderDisplayName(provider))}</strong>
          <small>${t("agentManagementProviderHint")}</small>
          <div class="pill-row">
            <span class="pill">${escapeHtml(providers.displayName(provider.provider_id))}</span>
            <span class="pill">${escapeHtml(environment.installed ? t("installed") : t("notDetected"))}</span>
            <span class="pill">${escapeHtml(environment.install_method || t("unknown"))}</span>
          </div>
        </div>
        <button
          type="button"
          data-action="detect-agent-provider"
          data-provider="${escapeAttr(provider.provider_id)}"
        >${t("detect")}</button>
      </header>
      <div class="stack">
        <div class="section-heading">
          <div>
            <strong>${t("agentManagementEnvironment")}</strong>
            <small>${t("agentManagementEnvironmentHint")}</small>
          </div>
        </div>
        <div class="manager-summary-grid agent-environment-grid">
          ${renderMetaLine(t("agentInstallStatus"), environment.installed ? t("installed") : t("notDetected"))}
          ${renderMetaLine(t("agentInstallMethod"), environment.install_method || t("unknown"))}
        </div>
        <div class="settings-list agent-environment-paths">
          ${renderAgentDetailRow(t("agentExecutablePath"), environment.executable_path || "—", t("agentExecutablePathHint"))}
          ${renderAgentDetailRow(t("agentExecutableDir"), environment.executable_dir || "—", t("agentExecutableDirHint"))}
          ${renderAgentDetailRow(t("agentConfigPath"), environment.config_path || "—", t("agentConfigPathHint"))}
        </div>
      </div>
      ${renderAgentHookStatus(provider)}
      <div class="stack">
        <div class="section-heading">
          <div>
            <strong>${t("agentProviderItems")}</strong>
            <small>${t("agentProviderItemsHint")}</small>
          </div>
        </div>
        ${
          items.length
            ? `<div class="settings-list">${items
                .map((setting) =>
                  setting.kind === "toggle"
                    ? renderAgentToggleRow(provider, setting)
                    : renderAgentActionRow(provider, setting)
                )
                .join("")}</div>`
            : `<div class="empty-state">${t("agentProviderItemsEmpty")}</div>`
        }
        ${actionItems.map((setting) => renderAgentSettingResult(provider.provider_id, setting.id)).join("")}
      </div>
      </div>`;
  }

  function renderAgentRuntimeSessions(provider) {
    const sessions = (state.agents.hookRuntimeSessions || []).filter(
      (session) => session.provider === provider.provider_id && !["completed", "failed"].includes(session.status)
    );
    return `
      <div class="stack">
        <div class="section-heading">
          <div>
            <strong>Active Hook Runtime Sessions</strong>
            <small>Live sessions observed through provider hooks.</small>
          </div>
        </div>
        ${
          sessions.length
            ? `<div class="settings-list agent-environment-paths">${sessions.map(renderRuntimeSessionRow).join("")}</div>`
            : `<div class="empty-state">No active hook runtime sessions.</div>`
        }
      </div>`;
  }

  function renderAgentHookStatus(provider) {
    const hook = provider.hook || {};
    const diagnosis = provider.hook_diagnosis || {};
    const profile = provider.hook_profile || null;
    const capabilities = provider.hook_capabilities || {};
    const events = Array.isArray(profile?.events) ? profile.events : [];
    const blockingCount = events.filter((event) => event.blocking).length;
    const status = hook.status || "unsupported";
    const lastEvent = hook.last_event_at ? formatDate(hook.last_event_at) : "—";
    return `
      <div class="stack">
        <div class="section-heading">
          <div>
            <strong>${t("hooks")}</strong>
            <small>${t("agentHookSummaryHint")}</small>
          </div>
          <a class="button" href="/hooks" data-nav="/hooks">${t("openHooks")}</a>
        </div>
        <div class="manager-summary-grid agent-environment-grid">
          ${renderMetaLine("Hook status", status)}
          ${renderMetaLine("Hook capabilities", formatHookCapabilities(capabilities))}
          ${renderMetaLine("Hook format", profile ? hookFormatLabel(profile.format) : "—")}
          ${renderMetaLine("Events", events.length ? `${events.length} total / ${blockingCount} blocking` : "—")}
          ${renderMetaLine("Installed version", hook.installed_version || "—")}
          ${renderMetaLine("Current version", hook.current_version || "—")}
          ${renderMetaLine("Last event", lastEvent)}
          ${renderMetaLine("Sessions", String(diagnosis.total_sessions || 0))}
          ${renderMetaLine("Linked", String(diagnosis.linked || 0))}
          ${renderMetaLine("Weak", String(diagnosis.weakly_linked || 0))}
          ${renderMetaLine("Attention", String((diagnosis.hook_needs_attention || 0) + (diagnosis.no_session_match || 0) + (diagnosis.no_active_runtime || 0) + (diagnosis.no_events_yet || 0) + (diagnosis.hook_not_installed || 0)))}
          ${renderMetaLine("Active runtime", String(diagnosis.active_runtime_sessions || 0))}
        </div>
        <div class="settings-list agent-environment-paths">
          ${renderAgentDetailRow("Hook config", hook.config_path || profile?.config_hint || "—", hook.message || "No hook status message.")}
          ${renderHookDiagnosisSummaryRow(diagnosis)}
          ${events.length ? renderHookEventProfileRow(events) : ""}
        </div>
      </div>`;
  }

  function renderHookDiagnosisSummaryRow(diagnosis) {
    if (!diagnosis || !diagnosis.total_sessions) {
      return `
        <div class="settings-row agent-detail-row">
          <div class="settings-copy settings-copy-inline">
            <strong>Session diagnosis</strong>
            <span>No scanned sessions for provider-level hook diagnosis.</span>
          </div>
          <div class="pill-row">
            <span class="pill">sessions=0</span>
          </div>
        </div>`;
    }
    return `
      <div class="settings-row agent-detail-row">
        <div class="settings-copy settings-copy-inline">
          <strong>Session diagnosis</strong>
          <span>Aggregated hook linkage and runtime health across scanned sessions.</span>
        </div>
        <div class="pill-row hook-event-pill-row">
          <span class="pill">linked=${escapeHtml(String(diagnosis.linked || 0))}</span>
          <span class="pill">weak=${escapeHtml(String(diagnosis.weakly_linked || 0))}</span>
          <span class="pill">needs_attention=${escapeHtml(String(diagnosis.hook_needs_attention || 0))}</span>
          <span class="pill">no_match=${escapeHtml(String(diagnosis.no_session_match || 0))}</span>
          <span class="pill">no_active=${escapeHtml(String(diagnosis.no_active_runtime || 0))}</span>
          <span class="pill">no_events=${escapeHtml(String(diagnosis.no_events_yet || 0))}</span>
        </div>
      </div>`;
  }

  function renderAgentDiagnosisActions(providerId, diagnosis) {
    const actions = diagnosis?.recommended_actions || [];
    if (!actions.length) return "";
    return `
      <div class="settings-row agent-detail-row">
        <div class="settings-copy settings-copy-inline">
          <strong>Recommended actions</strong>
          <span>Suggested follow-up based on aggregated session hook diagnosis.</span>
        </div>
        <div class="pill-row">
          ${actions.map((action) => {
            const pending = !!state.agents.pendingSettings[`${providerId}:${action.setting_id}`];
            return `<button
              type="button"
              data-action="run-agent-setting"
              data-provider="${escapeAttr(providerId)}"
              data-setting-id="${escapeAttr(action.setting_id)}"
              title="${escapeAttr(action.reason || "")}"
              ${pending ? "disabled" : ""}
            >${escapeHtml(pending ? t("running") : action.label)}</button>`;
          }).join("")}
        </div>
      </div>`;
  }

  function hookFormatLabel(format) {
    return String(format || "unknown")
      .split("_")
      .map((part) => (part ? part[0].toUpperCase() + part.slice(1) : part))
      .join(" ");
  }

  function formatHookCapabilities(capabilities) {
    const enabled = ["detect", "verify", "install", "repair", "uninstall"].filter(
      (key) => capabilities[key] === true
    );
    return enabled.length ? enabled.join(" / ") : "—";
  }

  function renderHookEventProfileRow(events) {
    return `
      <div class="settings-row agent-detail-row">
        <div class="settings-copy settings-copy-inline">
          <strong>Hook events</strong>
          <span>Provider events memorph can capture. Blocking events may require a policy decision.</span>
        </div>
        <div class="pill-row hook-event-pill-row">
          ${events
            .map((event) => `<span class="pill" title="${escapeAttr(event.blocking ? "blocking" : "record-only")}">${escapeHtml(event.name)}${event.blocking ? " *" : ""}</span>`)
            .join("")}
        </div>
      </div>`;
  }

  function renderAgentDetailRow(label, value, hint) {
    return `
      <div class="settings-row agent-detail-row">
        <div class="settings-copy settings-copy-inline">
          <strong>${escapeHtml(label)}</strong>
          <span>${escapeHtml(hint)}</span>
        </div>
        <div class="path-line">${escapeHtml(String(value || "—"))}</div>
      </div>`;
  }

  function renderAgentToggleRow(provider, setting) {
    return `
      <div class="settings-row">
        <div class="settings-copy settings-copy-inline">
          <strong>${escapeHtml(agentSettingLabel(setting))}</strong>
          <span>${escapeHtml(setting.description || "")}</span>
        </div>
        <label class="settings-check">
          <input
            type="checkbox"
            data-role="agent-setting-toggle"
            data-provider="${escapeAttr(provider.provider_id)}"
            data-setting-id="${escapeAttr(setting.id)}"
            ${setting.value ? "checked" : ""}
          >
          <span>${setting.value ? t("enabled") : t("disabled")}</span>
        </label>
      </div>`;
  }

  function renderAgentActionRow(provider, setting) {
    const pending = !!state.agents.pendingSettings[`${provider.provider_id}:${setting.id}`];
    const label = agentSettingLabel(setting);
    return `
      <div class="settings-row">
        <div class="settings-copy settings-copy-inline">
          <strong>${escapeHtml(label)}</strong>
          <span>${escapeHtml(setting.description || "")}</span>
        </div>
        <button
          type="button"
          class="invert"
          data-action="run-agent-setting"
          data-provider="${escapeAttr(provider.provider_id)}"
          data-setting-id="${escapeAttr(setting.id)}"
          ${pending ? "disabled" : ""}
        >${escapeHtml(pending ? t("running") : label)}</button>
      </div>`;
  }

  function renderAgentSettingResult(providerId, settingId) {
    const result = state.agents.settingResults[`${providerId}:${settingId}`];
    if (!result) return "";
    if (settingId === "repair_workspace_sessions") {
      return renderCodexRepairReport(result);
    }
    if (result.type === "hook_operation") {
      return renderHookOperationReport(result);
    }
    return `<pre class="code-block">${escapeHtml(JSON.stringify(result, null, 2))}</pre>`;
  }

  function renderHookOperationReport(result) {
    const report = result?.type === "hook_operation" ? result.data : null;
    if (!report) return "";
    const status = report.status || {};
    return `
      <section class="manager-workspace-summary">
        <div class="manager-summary-grid agent-environment-grid">
          ${renderMetaLine("Provider", report.provider)}
          ${renderMetaLine("Operation", report.operation)}
          ${renderMetaLine("Changed", report.changed ? "yes" : "no")}
          ${renderMetaLine("Status", status.status || "unknown")}
          ${renderMetaLine("Installed version", status.installed_version || "—")}
          ${renderMetaLine("Current version", status.current_version || "—")}
          ${renderMetaLine("Last event", status.last_event_at ? formatDate(status.last_event_at) : "—")}
          ${status.config_path ? renderWideMetaLine("Hook config", status.config_path) : ""}
          ${report.backup_path ? renderWideMetaLine("Backup", report.backup_path) : ""}
        </div>
        ${report.message || status.message ? `<div class="empty-state">${escapeHtml(report.message || status.message)}</div>` : ""}
      </section>`;
  }

  function renderCodexRepairReport(result) {
    const report = result?.type === "codex_workspace_repair" ? result.data : null;
    if (!report) {
      return "";
    }
    const touched = report.touched_sessions || [];
    return `
      <section class="manager-workspace-summary">
        <div class="manager-summary-grid codex-repair-summary-grid">
          ${renderMetaLine(t("workspace"), report.workspace_dir)}
          ${renderMetaLine(t("currentProvider"), report.current_model_provider)}
          ${renderMetaLine(t("scanned"), String(report.scanned_rollouts))}
          ${renderMetaLine(t("workspaceSessions"), String(report.workspace_session_count))}
          ${renderMetaLine(t("hiddenSessions"), String(report.hidden_session_count))}
          ${renderCodexRepairCountLine(report.repaired_session_count, touched.length)}
          ${renderMetaLine(t("reindexedSessions"), String(report.reindexed_session_count))}
          ${renderMetaLine(t("updatedSqliteRows"), String(report.sqlite_rows_updated || 0))}
          ${report.backup_dir ? renderWideMetaLine(t("backupLocation"), report.backup_dir) : ""}
          ${report.pruned_backup_count ? renderMetaLine(t("prunedBackups"), String(report.pruned_backup_count)) : ""}
          ${report.skipped_rollout_files?.length ? renderMetaLine(t("skippedRollouts"), String(report.skipped_rollout_files.length)) : ""}
        </div>
        ${touched.length ? "" : `<div class="empty-state">${t("noRepairNeeded")}</div>`}
      </section>`;
  }

  function renderCodexRepairCountLine(count, touchedCount) {
    if (!touchedCount) {
      return renderMetaLine(t("repairedSessions"), String(count || 0));
    }
    return `
      <div class="stack">
        <span class="eyebrow">${escapeHtml(t("repairedSessions"))}</span>
        <button type="button" class="meta-link-button" data-action="open-repaired-sessions">
          ${escapeHtml(String(count || 0))}
        </button>
      </div>`;
  }

  function renderWideMetaLine(label, value) {
    if (value === null || value === undefined || value === "") return "";
    return `<div class="stack meta-line-wide"><span class="eyebrow">${escapeHtml(label)}</span><div class="path-line">${escapeHtml(
      formatValue(value)
    )}</div></div>`;
  }

  function openRepairedSessionsModal() {
    const result = state.agents.settingResults["codex:repair_workspace_sessions"];
    const report = result?.type === "codex_workspace_repair" ? result.data : null;
    const touched = report?.touched_sessions || [];
    state.modal = {
      kind: "info",
      title: t("repairedSessions"),
      subtitle: t("repairedSessionsHint"),
      body: touched.length
        ? `<div class="session-list repaired-session-list">
            ${touched
              .map(
                (item) => `
                  <article class="session-row">
                    <div class="session-row-main">
                      <div class="session-info">
                        <div class="session-title-line">
                          <span class="session-title">${escapeHtml(item.title || item.session_id)}</span>
                          <span class="session-workspace">${escapeHtml(item.current_model_provider || "—")}</span>
                        </div>
                        <div class="session-meta-bar">
                          <span class="session-id-pill">${escapeHtml(item.session_id)}</span>
                          <span class="meta-dot">·</span>
                          <span class="meta-item">${escapeHtml(item.previous_model_provider || "—")} → ${escapeHtml(
                            item.current_model_provider || "—"
                          )}</span>
                          <span class="meta-dot">·</span>
                          <span class="meta-item">${escapeHtml(item.added_to_index ? t("reindexed") : t("providerUpdated"))}</span>
                        </div>
                      </div>
                    </div>
                  </article>`
              )
              .join("")}
          </div>`
        : `<div class="empty-state">${t("noRepairNeeded")}</div>`,
    };
    render();
  }

  function agentSettingLabel(setting) {
    switch (setting.id) {
      case "repair_workspace_sessions":
        return t("repairCurrentWorkspaceSessions");
      case "show_subagents":
        return t("showSubagents");
      case "install_hook":
        return t("hookInstall");
      case "verify_hook":
        return t("hookVerify");
      case "repair_hook":
        return t("hookRepair");
      case "uninstall_hook":
        return t("hookUninstall");
      default:
        return setting.title || setting.id;
    }
  }

  function agentProviderDisplayName(provider) {
    return providers.displayName(provider);
  }

  function agentProviderEnvironment(provider) {
    return provider?.environment || {
      installed: provider?.installed || false,
      executable_path: provider?.executable_path || null,
      executable_dir: provider?.executable_dir || null,
      config_path: provider?.config_path || "",
      install_method: provider?.install_method || "",
    };
  }

  function renderRuntimeSessionRow(session) {
    const status = `${session.status || "unknown"}${session.workspace_dir ? ` · ${workspaceName(session.workspace_dir)}` : ""}`;
    const detail = session.provider_session_id || session.runtime_id || session.id || "—";
    return renderAgentDetailRow(detail, status, session.workspace_dir || session.created_at || "—");
  }

  function formatToolInputHint(input) {
    if (input === null || input === undefined) return "";
    if (typeof input === "string") {
      const trimmed = input.trim();
      return trimmed ? ` (${trimmed.slice(0, 48)}${trimmed.length > 48 ? "..." : ""})` : "";
    }
    try {
      const json = JSON.stringify(input);
      return json && json !== "{}" ? ` (${json.slice(0, 48)}${json.length > 48 ? "..." : ""})` : "";
    } catch {
      return "";
    }
  }

  return {
    agentSettingLabel,
    openRepairedSessionsModal,
    openSettingsModal,
    renderAgentDetailRow,
    renderAgentManagementPage,
  };
}
