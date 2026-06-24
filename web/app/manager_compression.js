export function createManagerCompressionModule({
  state,
  providers,
  t,
  escapeHtml,
  escapeAttr,
  formatDate,
  formatBytes,
  formatRatio,
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

  function renderManagerPreview(preview, report = null) {
    const rows = preview.items
      .map((item) => {
        const encoded = escapeAttr(encodeURIComponent(JSON.stringify(item)));
        const href = `/sessions/${encodeURIComponent(item.provider_id)}/${encodeURIComponent(item.session_id)}`;
        return `
          <article class="manager-row">
            <div class="manager-row-head">
              <div class="manager-row-copy">
                <a class="manager-title-link" href="${href}" data-nav="${href}">${escapeHtml(item.title || item.session_id)}</a>
                <div class="manager-meta">
                  <span>${escapeHtml(providers.displayName(item.provider_id))}</span>
                  <span>${escapeHtml(formatBytes(item.size_bytes))}</span>
                  <span>${escapeHtml(t("managerUpdatedAt").replace("{time}", formatDate(item.last_active_at)))}</span>
                </div>
              </div>
              <label class="manager-select">
                <input type="checkbox" name="manager_item" value="${encoded}">
              </label>
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
          <label class="check-row">
            <input type="checkbox" data-role="select-all-manager">
            <span>${t("selectAll")}</span>
          </label>
        </div>
      </div>
      <div class="manager-list">${rows || `<div class="empty-state">${t("emptySessions")}</div>`}</div>`;
  }

  function updateManagerSelectionStats() {
    const preview = state.manager.preview;
    const summary = document.querySelector('[data-role="manager-selection-summary"]');
    if (!preview || !summary) return;
    const selected = selectedManagerItems();
    const bytes = selected.reduce((sum, item) => sum + Number(item.size_bytes || 0), 0);
    summary.textContent = managerSelectionSummary(preview.total_count, selected.length, bytes);
    const all = [...document.querySelectorAll('input[name="manager_item"]')];
    const selectAll = document.querySelector('input[data-role="select-all-manager"]');
    if (selectAll) {
      selectAll.checked = all.length > 0 && selected.length === all.length;
      selectAll.indeterminate = selected.length > 0 && selected.length < all.length;
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
    const preview = state.manager.preview;
    const report = state.manager.report;
    return `
      <div class="manager-page-layout">
        <section class="section-panel manager-control-panel">
          ${renderManagerForm(draft)}
        </section>
        <section class="section-panel manager-result-panel">
          ${renderManagerPreview(preview || emptyManagerPreview(), report)}
        </section>
      </div>`;
  }

  function renderCompressionProviderSupport(providers) {
    if (!providers.length) {
      return `<div class="manager-list agent-provider-list"><div class="empty-state">${t("noProviders")}</div></div>`;
    }
    return `
      <div class="manager-list agent-provider-list">
        ${providers
          .map((provider) => {
            const defaultProjection = provider.default_projection || "portable";
            return `
              <div class="agent-provider-item">
                <span class="agent-provider-head">
                  <strong class="agent-provider-name">${escapeHtml(providers.displayName(provider.provider_id))}</strong>
                  <span class="pill">${escapeHtml(defaultProjection)}</span>
                </span>
              </div>`;
          })
          .join("")}
      </div>`;
  }

  function renderCompressionPage() {
    const archives = state.compression.archives || [];
    const providers = state.compression.providers || [];
    const rows = archives
      .map((archive) => {
        const archiveRef = archive.archive_ref || "";
        return `
          <article class="manager-row">
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
                <button type="button" data-action="open-compression-restore" data-archive-ref="${escapeAttr(archiveRef)}">${t("restore")}</button>
              </div>
            </div>
          </article>`;
      })
      .join("");

    return `
      <div class="manager-page-layout">
        <section class="section-panel manager-control-panel agent-provider-panel">
          <section class="manager-workspace-summary">
            <div>
              <span class="eyebrow">${t("compression")}</span>
              <strong>${t("compressionTitle")}</strong>
              <p>${t("compressionHint")}</p>
            </div>
          </section>
          ${renderCompressionProviderSupport(providers)}
          <div class="manager-control-actions">
            <button type="button" data-action="open-compression-expand">${t("expand")}</button>
            <button class="invert" type="button" data-action="refresh-compression">${t("refresh")}</button>
          </div>
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
    renderManagerPage,
    unitOption,
    updateManagerSelectionStats,
  };
}
