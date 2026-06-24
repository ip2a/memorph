export function createModalModule({
  state,
  t,
  escapeHtml,
  escapeAttr,
  formatDate,
  workspaceName,
  render,
  getOrderedProviders,
  getDefaultSwitchTarget,
  homeProviderOptions,
  renderPathField,
  renderWorkspaceDatalist,
}) {
  function openImportModal() {
    state.modal = {
      kind: "form",
      title: t("import"),
      submit: "import-session",
      body: `
        <div class="stack">
          <label class="field">
            <span>${t("targetProvider")}</span>
            <select name="provider">${homeProviderOptions()}</select>
          </label>
          <label class="field">
            <span>${t("fileOrId")}</span>
            <input name="file_or_id" required placeholder="${escapeAttr(t("fileOrIdPlaceholder"))}">
          </label>
          ${renderPathField("to_dir", t("targetDir"), state.home.workspace, t("workspaceFieldHint"), state.home.workspace)}
        </div>
        ${renderWorkspaceDatalist()}`,
      submitLabel: t("import"),
    };
    render();
  }

  function openWorkspaceSwitchModal() {
    const items = (state.meta.workspaces || [])
      .map(
        (item) => `
        <div class="workspace-option workspace-switch-item">
          <button type="button" class="workspace-option-main" data-action="pick-workspace" data-workspace="${escapeAttr(item.path)}">
            <div class="workspace-option-head">
              <strong>${escapeHtml(workspaceName(item.path))}</strong>
              <span class="workspace-time">${escapeHtml(formatDate(item.last_viewed_at))}</span>
            </div>
            <div class="workspace-option-path">${escapeHtml(item.path)}</div>
          </button>
          <button type="button" class="ghost workspace-delete" data-action="delete-workspace-history" data-workspace="${escapeAttr(item.path)}">${t("remove")}</button>
        </div>`
      )
      .join("");
    state.modal = {
      kind: "form",
      title: t("switchWorkspace"),
      submit: "workspace-switch",
      body: `
        <div class="stack">
          <label class="field">
            <span>${t("workspacePath")}</span>
            <div class="path-picker">
              <input name="workspace" list="known-workspaces" value="${escapeAttr(state.home.workspace || "")}" placeholder="${escapeAttr(
                state.meta?.workspaces?.[0]?.path || ""
              )}">
              <button type="button" class="ghost" data-action="browse-folder" data-target-field="workspace">${t("browse")}</button>
            </div>
            <small class="muted">${t("workspaceFieldHint")}</small>
          </label>
          <div class="workspace-list workspace-switch-list">${items || `<div class="empty-state">${t("noWorkspace")}</div>`}</div>
        </div>
        ${renderWorkspaceDatalist()}`,
      submitLabel: t("go"),
    };
    render();
  }

  function openWorkspaceHistoryModal() {
    const items = (state.meta.workspaces || [])
      .map(
        (item) => `
        <button type="button" class="workspace-option" data-action="pick-workspace" data-workspace="${escapeAttr(item.path)}">
          <div class="workspace-option-head">
            <strong>${escapeHtml(workspaceName(item.path))}</strong>
            <span class="workspace-time">${escapeHtml(formatDate(item.last_viewed_at))}</span>
          </div>
          <div class="path-line">${escapeHtml(item.path)}</div>
        </button>`
      )
      .join("");
    state.modal = {
      kind: "custom",
      title: t("workspaceHistory"),
      body: `
        <div class="stack">
          <p class="muted">${t("workspaceHistoryHint")}</p>
          <div class="workspace-list">${items || `<div class="empty-state">${t("noWorkspace")}</div>`}</div>
          <div class="modal-actions">
            <button type="button" class="invert" data-action="close-modal">${t("done")}</button>
          </div>
        </div>`,
    };
    render();
  }

  function openSortOptionsModal() {
    state.modal = {
      kind: "form",
      title: t("listOptions"),
      submit: "home-list-options",
      body: `
        <div class="stack">
          <label class="field">
            <span>${t("sort")}</span>
            <select name="sort">
              <option value="recent" ${state.home.sort === "recent" ? "selected" : ""}>${t("recentFirst")}</option>
              <option value="title" ${state.home.sort === "title" ? "selected" : ""}>${t("titleAsc")}</option>
              <option value="hook_attention" ${state.home.sort === "hook_attention" ? "selected" : ""}>${t("hookAttentionFirst")}</option>
            </select>
          </label>
          <label class="field">
            <span>${t("hookFilter")}</span>
            <select name="hook_filter">
              <option value="all" ${state.home.hookFilter === "all" ? "selected" : ""}>${t("allHookStates")}</option>
              <option value="attention" ${state.home.hookFilter === "attention" ? "selected" : ""}>${t("hookFilterAttention")}</option>
              <option value="weak" ${state.home.hookFilter === "weak" ? "selected" : ""}>${t("hookFilterWeak")}</option>
              <option value="runtime" ${state.home.hookFilter === "runtime" ? "selected" : ""}>${t("hookFilterRuntime")}</option>
              <option value="linked" ${state.home.hookFilter === "linked" ? "selected" : ""}>${t("hookFilterLinked")}</option>
              <option value="no_hook" ${state.home.hookFilter === "no_hook" ? "selected" : ""}>${t("hookFilterNoHook")}</option>
              <option value="no_match" ${state.home.hookFilter === "no_match" ? "selected" : ""}>${t("hookFilterNoMatch")}</option>
            </select>
          </label>
          <label class="field">
            <span>${t("visible")}</span>
            <input type="number" min="1" max="200" name="visible" value="${state.home.visible}">
          </label>
        </div>`,
      submitLabel: t("apply"),
    };
    render();
  }

  function openAgentFilterModal() {
    const items = getOrderedProviders()
      .map((item) => {
        const checked = state.home.providers.includes(item.id);
        return `
          <label class="agent-pill">
            <input data-role="provider-toggle" type="checkbox" value="${escapeAttr(item.id)}" ${checked ? "checked" : ""}>
            <span>${escapeHtml(item.name)}</span>
          </label>`;
      })
      .join("");
    state.modal = {
      view: "agent-filter",
      kind: "custom",
      title: t("terminalAgents"),
      body: `
        <div class="stack">
          <div class="agent-more-list modal-agent-list">${items || `<div class="empty-state">${t("emptySessions")}</div>`}</div>
          <div class="modal-actions">
            <button type="button" class="invert" data-action="close-modal">${t("done")}</button>
          </div>
        </div>`,
    };
    render();
  }

  function openSwitchModal(provider, sessionId, workspace, currentTitle = "") {
    const defaultTarget = getDefaultSwitchTarget(provider);
    const initialTitle = (currentTitle || "").trim();
    state.modal = {
      kind: "form",
      title: t("copy"),
      submit: "switch-session",
      body: `
        <div class="copy-modal-grid">
          <input type="hidden" name="from" value="${escapeAttr(provider)}">
          <input type="hidden" name="session_id" value="${escapeAttr(sessionId)}">
          <label class="field copy-modal-target">
            <span>${t("targetProvider")}</span>
            <select name="to">${homeProviderOptions(provider, defaultTarget)}</select>
          </label>
          <label class="field copy-modal-title">
            <span>${t("copySessionTitleLabel")}</span>
            <input name="target_title" value="${escapeAttr(initialTitle)}" placeholder="${escapeAttr(initialTitle)}">
          </label>
          <div class="copy-modal-dir">
            ${renderPathField("to_dir", t("targetDir"), workspace || "", t("targetWorkspaceHint"))}
            <small class="muted copy-modal-remove-hint">${t("copySessionTitleHint")}</small>
          </div>
        </div>
        ${renderWorkspaceDatalist()}`,
      submitLabel: t("copyAction"),
      extraActions: [
        {
          label: t("moveAction"),
          submit: "switch-session",
          action: "move",
          className: "danger invert",
        },
      ],
    };
    render();
  }

  function openExportModal(provider, sessionId) {
    state.modal = {
      kind: "form",
      title: t("export"),
      submit: "export-session",
      body: `
        <input type="hidden" name="provider" value="${escapeAttr(provider)}">
        <input type="hidden" name="session_id" value="${escapeAttr(sessionId)}">
        <div class="stack">
          <label class="field">
            <span>${t("outputPrefix")}</span>
            <input name="output_prefix" value="${escapeAttr(sessionId)}">
          </label>
          <label class="field">
            <span>${t("format")}</span>
            <select name="format">
              <option value="json">json</option>
              <option value="md">md</option>
              <option value="html">html</option>
              <option value="morph">morph</option>
              <option value="both">both</option>
            </select>
          </label>
        </div>`,
      submitLabel: t("export"),
    };
    render();
  }

  function openCompressionRestoreModal(archiveRef) {
    const defaultPrefix = archiveRef
      .replace(/^memorph-archive:\/\//, "")
      .replace(/[^a-zA-Z0-9_-]+/g, "_")
      .replace(/^_+|_+$/g, "");
    state.modal = {
      kind: "form",
      title: t("restoreCompressionArchive"),
      submit: "restore-compression",
      body: `
        <input type="hidden" name="archive_ref" value="${escapeAttr(archiveRef)}">
        <div class="stack">
          <div class="verify-block">
            <span class="block-label">${t("archiveRef")}</span>
            <div class="path-line">${escapeHtml(archiveRef)}</div>
          </div>
          <label class="field">
            <span>${t("outputPrefix")}</span>
            <input name="output_prefix" value="${escapeAttr(defaultPrefix || "compression_archive")}">
          </label>
          <label class="field">
            <span>${t("format")}</span>
            <select name="format">
              <option value="json">json</option>
              <option value="md">md</option>
              <option value="html">html</option>
              <option value="morph">morph</option>
              <option value="both">both</option>
            </select>
          </label>
        </div>`,
      submitLabel: t("restore"),
    };
    render();
  }

  function openCompressionExpandModal() {
    state.modal = {
      kind: "form",
      title: t("expandCompressionSession"),
      submit: "expand-compression",
      body: `
        <div class="stack">
          <label class="field">
            <span>${t("sessionFile")}</span>
            <input name="file" placeholder="${escapeAttr(t("fileOrIdPlaceholder"))}">
          </label>
          <label class="field">
            <span>${t("outputPrefix")}</span>
            <input name="output_prefix" placeholder="session_expanded">
          </label>
          <label class="field">
            <span>${t("format")}</span>
            <select name="format">
              <option value="json">json</option>
              <option value="md">md</option>
              <option value="html">html</option>
              <option value="morph">morph</option>
              <option value="both">both</option>
            </select>
          </label>
        </div>`,
      submitLabel: t("expand"),
    };
    render();
  }

  function openRenameModal(provider, sessionId, title) {
    state.modal = {
      kind: "form",
      title: t("rename"),
      submit: "rename-session",
      body: `
        <input type="hidden" name="provider" value="${escapeAttr(provider)}">
        <input type="hidden" name="session_id" value="${escapeAttr(sessionId)}">
        <label class="field">
          <span>${t("title")}</span>
          <input name="title" required value="${escapeAttr(title)}">
        </label>`,
      submitLabel: t("save"),
    };
    render();
  }

  function openDeleteModal(provider, sessionId) {
    state.modal = {
      kind: "form",
      title: t("remove"),
      submit: "delete-session",
      body: `
        <input type="hidden" name="provider" value="${escapeAttr(provider)}">
        <input type="hidden" name="session_id" value="${escapeAttr(sessionId)}">
        <p>${t("deleteConfirm")}</p>`,
      submitLabel: t("remove"),
      submitClass: "danger",
    };
    render();
  }

  function openShareCreateModal(provider, sessionId, title) {
    const targetProviders = getOrderedProviders().filter((item) => item.id !== provider && item.export);
    const preferredTarget = getDefaultSwitchTarget(provider);
    const defaultTarget = targetProviders.some((item) => item.id === preferredTarget)
      ? preferredTarget
      : targetProviders[0]?.id || "";
    const options = targetProviders
      .map(
        (item) => `
        <label class="check-row">
          <input type="checkbox" name="targets" value="${escapeAttr(item.id)}"${
            item.id === defaultTarget ? " checked" : ""
          }>
          <span>${escapeHtml(item.name)}</span>
        </label>`
      )
      .join("");
    state.modal = {
      kind: "form",
      title: t("createShared"),
      submit: "create-shared",
      body: `
        <input type="hidden" name="provider" value="${escapeAttr(provider)}">
        <input type="hidden" name="session_id" value="${escapeAttr(sessionId)}">
        <div class="stack">
          <label class="field">
            <span>${t("title")}</span>
            <input name="title" value="${escapeAttr(title)}">
          </label>
          ${renderPathField("to_dir", t("targetDir"), state.home.workspace, t("workspaceFieldHint"))}
          <div class="field">
            <span>${t("targetProviders")}</span>
            <div class="check-grid">${options || `<div class="empty-state">${t("noSyncTargets")}</div>`}</div>
          </div>
        </div>
        ${renderWorkspaceDatalist()}`,
      submitLabel: t("create"),
    };
    render();
  }

  function openSharedRenameModal(groupId, title) {
    state.modal = {
      kind: "form",
      title: t("rename"),
      submit: "rename-shared",
      body: `
        <input type="hidden" name="group_id" value="${escapeAttr(groupId)}">
        <label class="field">
          <span>${t("title")}</span>
          <input name="title" value="${escapeAttr(title)}" required>
        </label>`,
      submitLabel: t("save"),
    };
    render();
  }

  function openSharedRemoveModal(groupId) {
    state.modal = {
      kind: "form",
      title: t("remove"),
      submit: "remove-shared",
      body: `
        <input type="hidden" name="group_id" value="${escapeAttr(groupId)}">
        <p>${t("removeSharedConfirm")}</p>
        <label class="check-row">
          <input type="checkbox" name="delete_provider_sessions" value="true">
          <span>${t("deleteProviderSessions")}</span>
        </label>`,
      submitLabel: t("remove"),
      submitClass: "danger",
    };
    render();
  }

  function openSharedBindModal(groupId) {
    state.modal = {
      kind: "form",
      title: t("addHolding"),
      submit: "bind-shared",
      body: `
        <input type="hidden" name="group_id" value="${escapeAttr(groupId)}">
        <div class="stack">
          <label class="field">
            <span>${t("provider")}</span>
            <select name="provider">${homeProviderOptions()}</select>
          </label>
          <label class="field">
            <span>${t("sessionId")}</span>
            <input name="session_id" placeholder="${escapeAttr(t("emptyCreatesNewHolding"))}">
          </label>
          ${renderPathField("to_dir", t("targetDir"), state.home.workspace, t("workspaceFieldHint"))}
        </div>
        ${renderWorkspaceDatalist()}`,
      submitLabel: t("addHolding"),
    };
    render();
  }

  function openPushSyncModal(groupId, holdingId, provider, sessionId) {
    state.modal = {
      kind: "form",
      title: t("pushSyncTitle"),
      submit: "sync-from-shared",
      body: `
        <input type="hidden" name="group_id" value="${escapeAttr(groupId)}">
        <input type="hidden" name="holding_id" value="${escapeAttr(holdingId)}">
        <div class="stack">
          <p class="muted">${t("pushSyncHint")}</p>
          <div class="path-line">${escapeHtml(provider)} / ${escapeHtml(sessionId)}</div>
        </div>`,
      submitLabel: t("syncFromThis"),
      submitClass: "invert",
    };
    render();
  }

  function openUnbindModal(groupId, holdingId, provider, sessionId) {
    state.modal = {
      kind: "form",
      title: t("unbind"),
      submit: "unbind-shared",
      body: `
        <input type="hidden" name="group_id" value="${escapeAttr(groupId)}">
        <input type="hidden" name="holding_id" value="${escapeAttr(holdingId)}">
        <div class="stack">
          <p>${t("unbindHint")}</p>
          <div class="path-line">${escapeHtml(provider)} / ${escapeHtml(sessionId)}</div>
        </div>`,
      submitLabel: t("unbind"),
      submitClass: "danger",
    };
    render();
  }

  function openSyncResultModal(report) {
    const successLines = (report.success || []).map((item) => `<div class="path-line">${escapeHtml(item)}</div>`).join("");
    const errorLines = (report.errors || []).map((item) => `<div class="path-line">${escapeHtml(item)}</div>`).join("");
    state.modal = {
      kind: "custom",
      title: t("syncComplete"),
      body: `
        <div class="stack">
          <div class="status-box stack">
            <strong>${t("syncComplete")}</strong>
            <small>${escapeHtml(report.source_provider || "")}</small>
          </div>
          <div class="detail-panel stack">
            <div>
              <span class="eyebrow">${t("success")}</span>
              <div class="stack">${successLines || `<div class="muted">0</div>`}</div>
            </div>
            <div>
              <span class="eyebrow">${t("errors")}</span>
              <div class="stack">${errorLines || `<div class="muted">0</div>`}</div>
            </div>
          </div>
          <div class="modal-actions">
            <button type="button" class="invert" data-action="close-modal">${t("done")}</button>
          </div>
        </div>`,
    };
    render();
  }

  function openActionResultModal({ title, summary = "", lines = [], navPath = "", navLabel = "" }) {
    state.modal = {
      kind: "custom",
      title,
      body: `
        <div class="stack">
          <div class="success-callout">
            <strong>${escapeHtml(title)}</strong>
            ${summary ? `<p>${escapeHtml(summary)}</p>` : ""}
          </div>
          ${
            lines.length
              ? `<div class="verify-block">
                  <span class="block-label">${t("details")}</span>
                  <div class="stack">${lines.map((line) => `<div class="path-line">${escapeHtml(line)}</div>`).join("")}</div>
                </div>`
              : ""
          }
          <div class="modal-actions">
            <button type="button" data-action="close-modal">${t("close")}</button>
            ${navPath ? `<a class="button invert" href="${escapeAttr(navPath)}" data-nav="${escapeAttr(navPath)}">${escapeHtml(navLabel || t("openDetail"))}</a>` : ""}
          </div>
        </div>`,
    };
    render();
  }

  return {
    openActionResultModal,
    openAgentFilterModal,
    openCompressionExpandModal,
    openCompressionRestoreModal,
    openDeleteModal,
    openExportModal,
    openImportModal,
    openPushSyncModal,
    openRenameModal,
    openShareCreateModal,
    openSharedBindModal,
    openSharedRemoveModal,
    openSharedRenameModal,
    openSortOptionsModal,
    openSwitchModal,
    openSyncResultModal,
    openUnbindModal,
    openWorkspaceHistoryModal,
    openWorkspaceSwitchModal,
  };
}
