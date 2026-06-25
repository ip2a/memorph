export function createManagerCompressionModule({
  state,
  providers,
  t,
  escapeHtml,
  escapeAttr,
  formatDate,
  formatBytes,
  formatRatio,
  markdown,
  formatContent,
  workspaceName,
  getOrderedProviders,
  renderMetaLine,
  selectedManagerItems,
  defaultManagerDraft,
}) {
  function managerSelectionSummary(total, selected, bytes) {
    return t("managerSelectionSummary")
      .replace("{count}", String(total))
      .replace("{selected}", String(selected))
      .replace("{size}", formatBytes(bytes));
  }

  function managerWorkspaceSelectionSummary(total, selected, bytes) {
    return t("managerWorkspaceSelectionSummary")
      .replace("{count}", String(total))
      .replace("{selected}", String(selected))
      .replace("{size}", formatBytes(bytes));
  }

  function renderManagerPreview(preview, report = null) {
    const selected = state.manager?.selectedItems || new Set();
    const rows = preview.items
      .map((item) => {
        const encoded = escapeAttr(encodeURIComponent(JSON.stringify(item)));
        const isSelected = selected.has(encoded);
        const href = `/sessions/${encodeURIComponent(item.provider_id)}/${encodeURIComponent(item.session_id)}`;
        return `
          <article class="manager-row ${isSelected ? "is-selected" : ""}" data-action="toggle-manager-item" data-value="${encoded}">
            <div class="manager-row-head">
              <div class="manager-row-copy">
                <a class="manager-title-link" href="${href}" data-nav="${href}">${escapeHtml(item.title || item.session_id)}</a>
                <div class="manager-meta">
                  <span>${escapeHtml(providers.displayName(item.provider_id))}</span>
                  <span>${escapeHtml(formatBytes(item.size_bytes))}</span>
                  <span>${escapeHtml(t("managerUpdatedAt").replace("{time}", formatDate(item.last_active_at)))}</span>
                </div>
              </div>
            </div>
          </article>`;
      })
      .join("");
    const reportSection = report
      ? `
        <div class="success-callout">
          <strong>${escapeHtml(report.title)}</strong>
          <p>${escapeHtml(report.summary)}</p>
        </div>
        ${
          report.lines.length
            ? `<div class="verify-block">
                <span class="block-label">${escapeHtml(report.toggleLabel)}</span>
                <div class="stack">${report.lines.map((line) => `<div class="path-line">${escapeHtml(line)}</div>`).join("")}</div>
              </div>`
            : ""
        }
        `
      : "";
    return `
      ${reportSection}
      <div class="section-heading manager-section-head">
        <div>
          <strong>${t("managerPreview")}</strong>
          <span class="manager-selection-summary" data-role="manager-selection-summary">
            ${managerSelectionSummary(preview.total_count, 0, 0)}
          </span>
        </div>
        <div class="manager-preview-actions">
          <button type="button" class="invert" data-action="open-manager-filter">${t("filters")}</button>
          <button type="button" class="danger" data-action="open-manager-clean-confirm">${t("cleanSelected")}</button>
          <button type="button" data-action="open-manager-backup-confirm">${t("backupSelected")}</button>
          <button type="button" class="button-small" data-action="select-all-manager">${t("selectAll")}</button>
        </div>
      </div>
      <div class="manager-list">${rows || `<div class="empty-state">${t("emptySessions")}</div>`}</div>`;
  }

  function renderManagerWorkspacePreview(preview, loading = false) {
    if (loading) {
      return `
        <div class="section-heading manager-section-head">
          <div><strong>${t("managerWorkspacePreview")}</strong></div>
        </div>
        <div class="manager-list"><div class="empty-state">${inlineSpinner()}</div></div>`;
    }
    const selected = state.manager?.selectedWorkspaceItems || new Set();
    const rows = (preview?.items || [])
      .map((item) => {
        const encoded = escapeAttr(encodeURIComponent(JSON.stringify(item)));
        const isSelected = selected.has(encoded);
        const href = `/manager?provider=${encodeURIComponent(item.provider_id)}&workspace=${encodeURIComponent(item.workspace)}`;
        return `
          <article class="manager-row ${isSelected ? "is-selected" : ""}" data-action="toggle-manager-workspace-item" data-value="${encoded}">
            <div class="manager-row-head">
              <div class="manager-row-copy">
                <a class="manager-title-link" href="${href}" data-nav="${href}">${escapeHtml(workspaceName(item.workspace) || item.workspace)}</a>
                <div class="manager-meta">
                  <span>${escapeHtml(providers.displayName(item.provider_id))}</span>
                  <span>${escapeHtml(t("workspaceSessionCount").replace("{count}", String(item.session_count || 0)))}</span>
                  <span>${escapeHtml(formatBytes(item.total_size_bytes))}</span>
                  <span>${escapeHtml(t("managerUpdatedAt").replace("{time}", formatDate(item.last_active_at)))}</span>
                </div>
                <div class="path-line">${escapeHtml(item.workspace)}</div>
              </div>
            </div>
          </article>`;
      })
      .join("");
    const summary = managerWorkspaceSelectionSummary(preview?.total_count || preview?.items?.length || 0, 0, 0);
    return `
      <div class="section-heading manager-section-head">
        <div>
          <strong>${t("managerWorkspacePreview")}</strong>
          <span class="manager-selection-summary" data-role="manager-workspace-selection-summary">${summary}</span>
        </div>
        <div class="manager-preview-actions">
          <button type="button" class="invert" data-action="open-manager-filter">${t("filters")}</button>
          <button type="button" class="danger" data-action="open-manager-clean-workspace-confirm">${t("cleanSelected")}</button>
          <button type="button" data-action="open-manager-backup-workspace-confirm">${t("backupSelected")}</button>
          <button type="button" class="button-small" data-action="select-all-manager-workspace">${t("selectAll")}</button>
        </div>
      </div>
      <div class="manager-list">${rows || `<div class="empty-state">${t("emptySessions")}</div>`}</div>`;
  }

  function updateManagerSelectionStats() {
    const preview = state.manager.preview;
    const summary = document.querySelector('[data-role="manager-selection-summary"]');
    if (preview && summary) {
      const selected = typeof selectedManagerItems === "function" ? selectedManagerItems() : [];
      const bytes = selected.reduce((sum, item) => sum + Number(item.size_bytes || 0), 0);
      summary.textContent = managerSelectionSummary(preview.total_count, selected.length, bytes);
      const all = [...document.querySelectorAll('.manager-row[data-action="toggle-manager-item"]')];
      const selectAll = document.querySelector('[data-action="select-all-manager"]');
      if (selectAll) {
        selectAll.classList.toggle("invert", all.length > 0 && selected.length === all.length);
        selectAll.textContent = all.length > 0 && selected.length === all.length ? t("deselectAll") : t("selectAll");
      }
    }

    const workspacePreview = state.manager.workspacePreview;
    const workspaceSummary = document.querySelector('[data-role="manager-workspace-selection-summary"]');
    if (workspacePreview && workspaceSummary) {
      const selected = selectedManagerWorkspaceItems();
      const bytes = selected.reduce((sum, item) => sum + Number(item.total_size_bytes || 0), 0);
      const total = workspacePreview.total_count || workspacePreview.items?.length || 0;
      workspaceSummary.textContent = managerWorkspaceSelectionSummary(total, selected.length, bytes);
      const all = [...document.querySelectorAll('.manager-row[data-action="toggle-manager-workspace-item"]')];
      const selectAll = document.querySelector('[data-action="select-all-manager-workspace"]');
      if (selectAll) {
        selectAll.classList.toggle("invert", all.length > 0 && selected.length === all.length);
        selectAll.textContent = all.length > 0 && selected.length === all.length ? t("deselectAll") : t("selectAll");
      }
    }
  }

  function emptyManagerPreview() {
    return {
      items: [],
      total_count: 0,
      total_size_bytes: 0,
    };
  }

  function renderManagerForm(managerDraft) {
    const providerChecks = getOrderedProviders()
      .filter((item) => !providers.isHiddenGlobal(item))
      .filter((item) => providers.hasFilter(item, "is_installed"))
      .map((item) => {
        const checked = managerDraft.providers.includes(item.provider_id);
        return `
          <label class="agent-provider-item manager-provider-item ${checked ? "is-active" : ""}">
            <input data-role="manager-provider-toggle" type="checkbox" name="manager_provider" value="${escapeAttr(item.provider_id)}" ${checked ? "checked" : ""}>
            <span class="agent-provider-head">
              <strong class="agent-provider-name">${escapeHtml(item.display_name)}</strong>
              <span class="agent-provider-state ${checked ? "is-installed" : "is-missing"}" aria-hidden="true">${checked ? "●" : "○"}</span>
            </span>
          </label>`;
      })
      .join("");
    return `
      <div class="manager-control-content">
        <section class="manager-workspace-summary">
          <div>
            <span class="eyebrow">${t("workspace")}</span>
            <strong>${escapeHtml(workspaceName(state.home.workspace) || t("workspaceEmpty"))}</strong>
            <p>${escapeHtml(state.home.workspace || "—")}</p>
          </div>
        </section>
        <section class="manager-control-bottom">
          <section class="stack">
            <div class="section-heading">
              <div>
                <strong>${t("providers")}</strong>
              </div>
            </div>
            <div class="manager-list agent-provider-list manager-provider-list">${providerChecks}</div>
          </section>
        </section>
      </div>`;
  }

  function renderManagerPage() {
    const draft = state.manager.draft || defaultManagerDraft();
    const isDefault = state.manager.isDefaultPreview !== false;
    const preview = isDefault ? state.manager.quickPreview : state.manager.preview;
    const workspacePreview = isDefault ? state.manager.quickWorkspacePreview : state.manager.workspacePreview;
    const report = state.manager.report;
    const viewMode = state.manager.viewMode || "sessions";
    const viewDashboard = renderManagerDashboard(state.manager.stats, viewMode, state.manager.statsLoading);
    return `
      <div class="manager-page-layout">
        <section class="section-panel manager-control-panel">
          ${renderManagerForm(draft)}
        </section>
        <section class="section-panel manager-result-panel">
          ${viewDashboard}
          ${viewMode === "workspaces"
            ? renderManagerWorkspacePreview(workspacePreview, workspacePreview === null)
            : renderManagerPreview(preview || emptyManagerPreview(), report)}
        </section>
      </div>`;
  }

  function renderManagerDashboard(stats, viewMode, statsLoading = false) {
    const isAllWorkspaces = viewMode === "workspaces";
    const pending = statsLoading || !stats;
    const primaryLabel = isAllWorkspaces ? t("managerAllWorkspaceScope") : t("managerCurrentWorkspaceScope");
    const primarySize = isAllWorkspaces
      ? stats?.all_workspace_size_bytes
      : stats?.current_workspace_size_bytes;
    const workspaceCount = stats?.all_workspace_count;
    const selectedAgentCount = stats?.selected_agent_count;
    const sessionCount = isAllWorkspaces
      ? stats?.all_workspace_session_count
      : stats?.current_workspace_session_count;
    const statValue = (value, formatter = (item) => String(item)) => {
      if (value != null) return escapeHtml(formatter(value));
      return pending ? inlineSpinner() : "—";
    };

    return `
      <div class="manager-view-tabs manager-stats-dashboard" aria-label="${escapeAttr(t("managerStatsDashboard"))}">
        <button type="button" class="manager-stat-card ${!isAllWorkspaces ? "is-active" : ""}" data-action="set-manager-view" data-view="sessions">
          <span>${escapeHtml(primaryLabel)}</span>
          <strong>${statValue(primarySize, formatBytes)}</strong>
          <em>${t("size")}</em>
        </button>
        <button type="button" class="manager-stat-card ${isAllWorkspaces ? "is-active" : ""}" data-action="set-manager-view" data-view="workspaces">
          <span>${t("managerAllWorkspaces")}</span>
          <strong>${statValue(workspaceCount)}</strong>
          <em>${t("managerWorkspaceUnit")}</em>
        </button>
        <div class="manager-stat-card is-readonly">
          <span>${t("managerSelectedAgents")}</span>
          <strong>${statValue(selectedAgentCount)}</strong>
          <em>${t("managerAgentUnit")}</em>
        </div>
        <div class="manager-stat-card is-readonly">
          <span>${t("managerCurrentSessions")}</span>
          <strong>${statValue(sessionCount)}</strong>
          <em>${t("sessionsStat")}</em>
        </div>
      </div>`;
  }

  function renderCompressionWorkspaceSummary() {
    const providerChecks = getOrderedProviders()
      .filter((item) => !providers.isHiddenGlobal(item))
      .filter((item) => providers.hasFilter(item, "is_installed"))
      .map((item) => {
        const checked = state.home.providers.includes(item.provider_id);
        return `
          <label class="agent-provider-item manager-provider-item ${checked ? "is-active" : ""}">
            <input data-role="provider-toggle" type="checkbox" value="${escapeAttr(item.provider_id)}" ${checked ? "checked" : ""}>
            <span class="agent-provider-head">
              <strong class="agent-provider-name">${escapeHtml(item.display_name)}</strong>
              <span class="agent-provider-state ${checked ? "is-installed" : "is-missing"}" aria-hidden="true">${checked ? "●" : "○"}</span>
            </span>
          </label>`;
      })
      .join("");
    return `
      <div class="manager-control-content">
        <section class="manager-workspace-summary">
          <div>
            <span class="eyebrow">${t("workspace")}</span>
            <strong>${escapeHtml(workspaceName(state.home.workspace) || t("workspaceEmpty"))}</strong>
            <p>${escapeHtml(state.home.workspace || "—")}</p>
          </div>
        </section>
        <section class="manager-control-bottom">
          <section class="stack">
            <div class="section-heading">
              <div>
                <strong>${t("providers")}</strong>
              </div>
            </div>
            <div class="manager-list agent-provider-list manager-provider-list">${providerChecks || `<div class="empty-state">${t("noProviders")}</div>`}</div>
          </section>
        </section>
      </div>`;
  }

  function renderBlock(block) {
    const text = block.text != null
      ? String(block.text)
      : block.content != null
        ? String(block.content)
        : block.diff_text != null
          ? String(block.diff_text)
          : block.summary != null
            ? String(block.summary)
            : JSON.stringify(block, null, 2);
    const formatted = formatContent(text);
    return `<div class="content-block">${formatted.html}</div>`;
  }

  function renderArchiveEvent(event, index) {
    const role = (event.role || "unknown").replaceAll("_", " ");
    const kind = (event.kind || "unknown").replaceAll("_", " ");
    const blocks = (event.blocks || []).map(renderBlock).join("") || `<p class="muted">${escapeHtml(t("noDetails"))}</p>`;
    return `
      <article class="msg-item" data-message-index="${index}" data-role="${escapeAttr(event.role || "unknown")}">
        <header class="msg-header">
          <span class="msg-header-main">
            <span class="msg-role">${escapeHtml(role)}</span>
            <span>${escapeHtml(kind)}</span>
          </span>
          <span class="msg-header-meta">
            <span>${escapeHtml(formatDate(event.timestamp))}</span>
          </span>
        </header>
        <div class="msg-body">${blocks}</div>
      </article>`;
  }

  function renderCompressionArchiveDetail(archive) {
    const events = archive.events || [];
    return `
      <div class="stack compression-archive-detail">
        <section class="section-panel stack">
          <div class="manager-summary-grid">
            ${renderMetaLine(t("archiveRef"), archive.archive_ref)}
            ${renderMetaLine(t("canonicalId") || "canonical id", archive.canonical_id)}
            ${renderMetaLine(t("sourceProvider"), archive.source_provider_id)}
            ${renderMetaLine(t("targetProvider"), archive.target_provider_id)}
            ${renderMetaLine(t("workspace"), archive.workspace_dir)}
            ${renderMetaLine(t("summaryEventId"), archive.summary_event_id)}
            ${renderMetaLine(t("sourceEvents"), String(archive.source_event_ids?.length ?? archive.source_event_count ?? events.length))}
            ${renderMetaLine(t("storedSize"), formatBytes(archive.stored_size_bytes))}
            ${renderMetaLine(t("originalSize"), formatBytes(archive.original_size_bytes))}
            ${renderMetaLine(t("createdAt"), formatDate(archive.created_at))}
          </div>
        </section>
        <section class="section-panel stack">
          <div class="section-heading">
            <div>
              <strong>${t("events")}</strong>
              <span>${events.length}</span>
            </div>
          </div>
          <div class="msg-list">
            ${events.length ? events.map(renderArchiveEvent).join("") : `<div class="empty-state">${t("noMessages")}</div>`}
          </div>
        </section>
      </div>`;
  }

  function renderCompressionPage() {
    const archives = state.compression.archives || [];
    const rows = archives
      .map((archive) => {
        const archiveRef = archive.archive_ref || "";
        return `
          <article class="manager-row" data-action="open-compression-detail" data-archive-ref="${escapeAttr(archiveRef)}">
            <div class="manager-row-head">
              <div class="manager-row-copy">
                <div class="session-title">${escapeHtml(archive.canonical_id || t("compressionArchive"))}</div>
                <div class="manager-meta">
                  <span>${escapeHtml(archive.source_provider_id || "—")} → ${escapeHtml(archive.target_provider_id || "—")}</span>
                  <span>${t("sourceEvents")}=${escapeHtml(String(archive.source_event_count ?? 0))}</span>
                  <span>${t("storedSize")}=${escapeHtml(formatBytes(archive.stored_size_bytes))}</span>
                  <span>${t("originalSize")}=${escapeHtml(formatBytes(archive.original_size_bytes))}</span>
                  <span>${t("compressionRatio")}=${escapeHtml(formatRatio(archive.compression_ratio))}</span>
                  <span>${t("createdAt")}=${escapeHtml(formatDate(archive.created_at))}</span>
                </div>
                <div class="path-line">${escapeHtml(archiveRef)}</div>
              </div>
              <div class="session-actions">
                <button type="button" data-action="open-compression-detail" data-archive-ref="${escapeAttr(archiveRef)}">${t("view")}</button>
                <button type="button" data-action="open-compression-restore" data-archive-ref="${escapeAttr(archiveRef)}">${t("restore")}</button>
              </div>
            </div>
          </article>`;
      })
      .join("");

    return `
      <div class="manager-page-layout">
        <section class="section-panel manager-control-panel">
          ${renderCompressionWorkspaceSummary()}
        </section>
        <section class="section-panel manager-result-panel">
          <div class="section-heading manager-section-head">
            <div>
              <strong>${t("compressionArchives")}</strong>
              <span>${archives.length}</span>
            </div>
          </div>
          <div class="manager-list">${rows || `<div class="empty-state">${t("emptyCompressionArchives")}</div>`}</div>
        </section>
      </div>`;
  }

  function unitOption(value, label, selected) {
    return `<option value="${escapeAttr(value)}" ${selected === value ? "selected" : ""}>${escapeHtml(label)}</option>`;
  }

  return {
    renderCompressionPage,
    renderCompressionArchiveDetail,
    renderManagerPage,
    unitOption,
    updateManagerSelectionStats,
    selectedManagerWorkspaceItems,
  };
}

function selectedManagerWorkspaceItems() {
  const visible = new Set(
    [...document.querySelectorAll('.manager-row[data-action="toggle-manager-workspace-item"]')].map(
      (el) => el.dataset.value
    )
  );
  return [...(state.manager?.selectedWorkspaceItems || new Set())]
    .filter((value) => visible.has(value))
    .map((value) => JSON.parse(decodeURIComponent(value)));
}
