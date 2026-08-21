import { useEffect, useState, type ReactNode } from "react";
import { CopyIcon, InfoIcon, Trash2Icon, ArchiveIcon } from "lucide-react";
import { toast } from "sonner";
import { PageError } from "@/components/shared/page-states";
import { ScrollPane } from "@/components/shared/scroll-pane";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";
import {
  Empty,
  EmptyHeader,
  EmptyTitle,
} from "@/components/ui/empty";
import { Spinner } from "@/components/ui/spinner";
import { SkillBundlePanel } from "@/features/skills/skill-bundle-panel";
import { SkillConsolidatePanel } from "@/features/skills/skill-consolidate-panel";
import { realPathOf, displayHomePath } from "@/features/skills/skills-real-path";
import { SkillContextHealthPanel } from "@/features/skills/skill-context-health-panel";
import { SkillCoverageConflictsPanel } from "@/features/skills/skill-coverage-conflicts-panel";
import { SkillHealthDetails } from "@/features/skills/skill-health-tags";
import { skillUsedByLabel } from "@/features/skills/skill-used-by-label";
import {
  skillInstallScopeKey,
  type SkillInstallScope,
} from "@/features/skills/skill-install-scope";
import { formatBytes, normalizeSkillDescription } from "@/lib/format";
import { useI18n } from "@/lib/i18n-context";
import { cn } from "@/lib/utils";
import { SectionHeading } from "@/components/shared/section-heading";
import type { I18nKey } from "@/lib/i18n-core";
import type { SkillAsset, SkillCatalogItem, SkillDetail, SkillFilePreview } from "@/lib/types";

const AGENTS = ["claude", "codex", "gemini", "opencode", "hermes"] as const;

type DetailTab = "bundle" | "coverage" | "installations" | "consolidate";

function DetailSection({
  title,
  description,
  className,
  children,
}: {
  title: string;
  description?: string;
  className?: string;
  children: ReactNode;
}) {
  return (
    <section className={cn("flex flex-col gap-4 border-t pt-5", className)}>
      <SectionHeading
        title={title}
        description={description}
        titleAs="h3"
        className="border-b-0 pb-0"
      />
      {children}
    </section>
  );
}

function installKindLabel(
  kind: SkillCatalogItem["installations"][number]["install_kind"],
  t: (key: I18nKey) => string,
) {
  switch (kind) {
    case "symlink":
      return t("skillsInstallKindSymlink");
    case "managed-copy":
      return t("skillsInstallKindManagedCopy");
    default:
      return t("skillsInstallKindDirectory");
  }
}

function ScopeInstallationRow({
  agent,
  scopeKind,
  workspaceDir,
  installation,
  expectedPath,
  pending,
  actionPending,
  sourceUsedBy,
  t,
  onInstall,
  onRemove,
}: {
  agent: (typeof AGENTS)[number];
  scopeKind: "global" | "project";
  workspaceDir?: string;
  installation?: SkillCatalogItem["installations"][number];
  expectedPath: string;
  pending: boolean;
  actionPending: boolean;
  sourceUsedBy?: string;
  t: (key: I18nKey, vars?: Record<string, string | number>) => string;
  onInstall: (scope: SkillInstallScope) => void;
  onRemove: (scope: SkillInstallScope) => void;
}) {
  const scope: SkillInstallScope = {
    usedBy: agent,
    scopeKind,
    workspaceDir: scopeKind === "project" ? workspaceDir : undefined,
  };
  const managed =
    installation?.install_kind === "symlink" ||
    installation?.install_kind === "managed-copy";
  const linkBroken =
    installation?.install_kind === "symlink" &&
    installation.link_status === "broken";
  const scopeLabel =
    scopeKind === "global"
      ? t("skillsAgentGlobalInstall")
      : t("skillsWorkspaceInstall");
  const enableLabel =
    scopeKind === "global"
      ? t("skillsEnableAgentSymlink")
      : t("skillsEnableWorkspaceSymlink");
  const removeLabel =
    scopeKind === "global"
      ? t("skillsRemoveAgentSymlink")
      : t("skillsRemoveWorkspaceSymlink");

  return (
    <div className="flex items-start justify-between gap-3 rounded-md border border-dashed p-3">
      <div className="min-w-0 flex-1 space-y-2">
        <div className="flex flex-wrap items-center gap-2">
          <span className="text-sm font-medium">{scopeLabel}</span>
          <Badge variant={installation ? "secondary" : "outline"}>
            {installation ? t("skillsEnabled") : t("skillsNotEnabled")}
          </Badge>
          {installation ? (
            <>
              <Badge variant="outline">
                {installKindLabel(installation.install_kind, t)}
              </Badge>
              {linkBroken ? (
                <Badge variant="destructive">{t("skillsLinkBroken")}</Badge>
              ) : null}
            </>
          ) : null}
        </div>
        <div className="space-y-1 text-xs">
          <span className="text-muted-foreground">
            {installation ? t("skillsInstallPath") : t("skillsExpectedInstallPath")}
          </span>
          <p className="break-all font-mono">
            {installation?.install_path ?? expectedPath}
          </p>
          {installation?.scope_kind === "project" && installation.workspace_dir ? (
            <div>
              <span className="text-muted-foreground">{t("skillsProjectScope")}</span>
              <p className="break-all font-mono">{installation.workspace_dir}</p>
            </div>
          ) : null}
          {installation?.install_kind === "symlink" && installation.symlink_target ? (
            <div>
              <span className="text-muted-foreground">{t("skillsSymlinkTarget")}</span>
              <p className="break-all font-mono">{installation.symlink_target}</p>
            </div>
          ) : null}
        </div>
      </div>
      {installation ? (
        <Button
          variant="destructive"
          size="sm"
          className="shrink-0"
          disabled={pending || !managed}
          title={!managed ? t("skillsUserOwnedHint") : undefined}
          onClick={() => onRemove(scope)}
        >
          {actionPending ? <Spinner data-icon="inline-start" /> : <Trash2Icon />}
          {removeLabel}
        </Button>
      ) : (
        <Button
          size="sm"
          className="shrink-0"
          disabled={pending || !sourceUsedBy}
          onClick={() => onInstall(scope)}
        >
          {actionPending ? <Spinner data-icon="inline-start" /> : null}
          {enableLabel}
        </Button>
      )}
    </div>
  );
}

function AgentInstallationSection({
  agent,
  installations,
  currentWorkspace,
  installationTargets,
  pending,
  pendingTarget,
  sourceUsedBy,
  t,
  onInstall,
  onRemove,
}: {
  agent: (typeof AGENTS)[number];
  installationTargets: SkillCatalogItem["installation_targets"];
  currentWorkspace?: string;
  pending: boolean;
  pendingTarget: string | null;
  sourceUsedBy?: string;
  t: (key: I18nKey, vars?: Record<string, string | number>) => string;
  onInstall: (scope: SkillInstallScope) => void;
  onRemove: (scope: SkillInstallScope) => void;
}) {
  const globalTarget = installationTargets.find(
    (target) => target.used_by === agent && target.scope_kind === "global",
  );
  const projectTarget = currentWorkspace
    ? installationTargets.find(
        (target) =>
          target.used_by === agent &&
          target.scope_kind === "project" &&
          target.workspace_dir === currentWorkspace,
      )
    : undefined;
  const globalInstallation = globalTarget?.installation ?? undefined;
  const projectInstallation = projectTarget?.installation ?? undefined;

  return (
    <div className="space-y-3 rounded-lg border p-3">
      <strong>{agent}</strong>
      <ScopeInstallationRow
        agent={agent}
        scopeKind="global"
        installation={globalInstallation}
        expectedPath={globalTarget?.expected_path ?? ""}
        pending={pending}
        actionPending={pendingTarget === skillInstallScopeKey({ usedBy: agent, scopeKind: "global" })}
        sourceUsedBy={sourceUsedBy}
        t={t}
        onInstall={onInstall}
        onRemove={onRemove}
      />
      {currentWorkspace ? (
        <ScopeInstallationRow
          agent={agent}
          scopeKind="project"
          workspaceDir={currentWorkspace}
          installation={projectInstallation}
          expectedPath={projectTarget?.expected_path ?? ""}
          pending={pending}
          actionPending={
            pendingTarget ===
            skillInstallScopeKey({
              usedBy: agent,
              scopeKind: "project",
              workspaceDir: currentWorkspace,
            })
          }
          sourceUsedBy={sourceUsedBy}
          t={t}
          onInstall={onInstall}
          onRemove={onRemove}
        />
      ) : null}
    </div>
  );
}

async function copyFingerprint(value: string, copied: string, failed: string) {
  try {
    await navigator.clipboard.writeText(value);
    toast.success(copied);
  } catch {
    toast.error(failed);
  }
}

// Always-visible summary of where this skill really lives and which symlinks
// point at it. The catalog row is keyed by canonical path, so the row IS the
// real skill; symlinks are installations pointing here, not separate entries.
function InstallationLocationSection({
  installations,
  usedBy,
  t,
}: {
  installations: SkillCatalogItem["installations"];
  usedBy: string[];
  t: (key: I18nKey, vars?: Record<string, string | number>) => string;
}) {
  const active = installations.filter((item) => item.status === "active");
  const realDirectory = active.find(
    (item) =>
      item.install_kind === "directory" || item.install_kind === "managed-copy",
  );
  const realPath =
    realDirectory?.install_path ??
    active.find((item) => item.install_kind === "symlink")?.symlink_target ??
    installations[0]?.install_path;
  const symlinks = active.filter((item) => item.install_kind === "symlink");
  const usesRealDirectly = active.some(
    (item) =>
      (item.install_kind === "directory" ||
        item.install_kind === "managed-copy") &&
      item.install_path === realPath,
  );

  return (
    <DetailSection title={t("skillsDetailsLocation")}>
      <dl className="space-y-4 text-sm">
        <div className="space-y-1.5">
          <dt className="text-muted-foreground text-xs font-medium">
            {t("skillsRealPath")}
          </dt>
          <dd className="break-all rounded-md border bg-muted/30 px-3 py-2 font-mono text-xs">
            {realPath ? displayHomePath(realPath) : "—"}
          </dd>
        </div>

        <div className="space-y-2">
          <dt className="text-muted-foreground text-xs font-medium">
            {t("skillsUsedBy")}
          </dt>
          <dd>
            {usedBy.length ? (
              <div className="flex flex-wrap gap-2">
                {usedBy.map((agent) => (
                  <Badge key={agent} variant="secondary">
                    {skillUsedByLabel(agent, t)}
                  </Badge>
                ))}
              </div>
            ) : (
              <p className="text-muted-foreground text-sm">{t("skillsNoUsedBy")}</p>
            )}
          </dd>
        </div>

        <div className="space-y-2">
          <dt className="text-muted-foreground text-xs font-medium">
            {t("skillsSymlinksHere", { count: symlinks.length })}
          </dt>
          <dd>
            {symlinks.length ? (
              <ul className="space-y-2">
                {symlinks.map((item) => (
                  <li
                    key={`${item.used_by}:${item.install_path}`}
                    className="space-y-1.5 rounded-md border px-3 py-2"
                  >
                    <div className="flex flex-wrap items-center gap-2">
                      <Badge variant="outline">
                        {skillUsedByLabel(item.used_by, t)}
                      </Badge>
                      {item.link_status === "broken" ? (
                        <Badge variant="destructive">{t("skillsLinkBroken")}</Badge>
                      ) : null}
                    </div>
                    <code className="block break-all font-mono text-xs">
                      {displayHomePath(item.install_path)}
                    </code>
                  </li>
                ))}
              </ul>
            ) : usesRealDirectly ? (
              <p className="text-muted-foreground text-sm">
                {t("skillsDirectDirectory")}
              </p>
            ) : (
              <p className="text-muted-foreground text-sm">{t("skillsNoSymlinks")}</p>
            )}
          </dd>
        </div>
      </dl>
    </DetailSection>
  );
}

export function SkillDetailPanel({
  selected,
  detail,
  assets,
  treeLoading,
  previewPath,
  onPreviewPathChange,
  preview,
  previewLoading,
  pending,
  pendingTarget,
  mutationError,
  provider,
  currentWorkspace,
  onInstall,
  onRemove,
  onDelete,
  onDisable,
  onConsolidate,
  onDeleteInstallation,
  onRemoveSymlinks,
}: {
  selected: SkillCatalogItem | null;
  detail?: SkillDetail;
  assets: SkillAsset[];
  treeLoading: boolean;
  previewPath: string | null;
  onPreviewPathChange: (path: string | null) => void;
  preview?: SkillFilePreview;
  previewLoading: boolean;
  pending: boolean;
  pendingTarget: string | null;
  mutationError: Error | null;
  provider?: string;
  currentWorkspace?: string;
  onInstall: (scope: SkillInstallScope) => void;
  onRemove: (scope: SkillInstallScope) => void;
  onDelete: () => void;
  onDisable: () => void;
  onConsolidate: (canonicalPath: string) => void;
  onDeleteInstallation: (installPath: string) => void;
  onRemoveSymlinks: () => void;
}) {
  const { t } = useI18n();
  const [tab, setTab] = useState<DetailTab>("bundle");
  const [detailsOpen, setDetailsOpen] = useState(false);
  const sourceUsedBy = selected?.installations.find(
    (item) => item.status === "active",
  )?.used_by;

  useEffect(() => {
    setDetailsOpen(false);
  }, [selected?.id]);

  if (!selected) {
    return (
      <Empty>
        <EmptyHeader>
          <EmptyTitle>{t("skillsEmpty")}</EmptyTitle>
        </EmptyHeader>
      </Empty>
    );
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-3">
      {mutationError ? (
        <PageError title={t("skillsActionFailed")} message={mutationError.message} />
      ) : null}
      <div className="flex shrink-0 flex-col gap-1">
        <div className="flex items-center justify-between gap-3">
          <h2 className="min-w-0 truncate text-xl font-semibold">{selected.name}</h2>
          <div className="flex shrink-0 flex-wrap justify-end gap-2">
            <Button
              variant="outline"
              size="sm"
              disabled={pending}
              title={t("skillsDisableHint")}
              onClick={onDisable}
            >
              <ArchiveIcon data-icon="inline-start" />
              {t("skillsDisable")}
            </Button>
            <Button
              variant="destructive"
              size="sm"
              disabled={pending}
              title={t("skillsDeleteHint")}
              onClick={onDelete}
            >
              <Trash2Icon data-icon="inline-start" />
              {t("delete")}
            </Button>
            <Dialog open={detailsOpen} onOpenChange={setDetailsOpen}>
              <DialogTrigger asChild>
                <Button variant="outline" size="sm" className="shrink-0">
                  <InfoIcon data-icon="inline-start" />
                  {t("skillsDetails")}
                </Button>
              </DialogTrigger>
            <DialogContent className="flex h-[min(85dvh,840px)] w-[min(960px,calc(100vw-2rem))] max-w-[calc(100vw-2rem)] flex-col gap-0 overflow-hidden p-0 sm:max-w-3xl">
              <DialogHeader className="shrink-0 border-b px-6 py-4">
                <DialogTitle>{selected.name}</DialogTitle>
                <DialogDescription>
                  {normalizeSkillDescription(selected.description) ||
                    t("skillsNoDescription")}
                </DialogDescription>
              </DialogHeader>
              <ScrollPane className="min-h-0 flex-1" innerClassName="px-6 py-4">
                <DetailSection title={t("skillsDetailsOverview")} className="border-t-0 pt-0">
                  <div className="flex flex-wrap gap-2">
                    <Badge variant="outline">
                      {t("skillsFilesCount", { count: selected.file_count })}
                    </Badge>
                    <Badge variant="outline">{formatBytes(selected.total_bytes)}</Badge>
                    {selected.version ? (
                      <Badge variant="outline">v{selected.version}</Badge>
                    ) : null}
                    {selected.missing ? (
                      <Badge variant="destructive">{t("skillsMissing")}</Badge>
                    ) : null}
                    {(detail?.tags ?? selected.tags).map((tag) => (
                      <Badge key={tag} variant="outline">
                        {tag}
                      </Badge>
                    ))}
                  </div>
                </DetailSection>
                <InstallationLocationSection
                  installations={selected.installations}
                  usedBy={detail?.used_by ?? selected.used_by}
                  t={t}
                />
                <DetailSection title={t("skillsFingerprint")}>
                  <div className="flex items-start gap-2">
                    <code className="min-w-0 flex-1 break-all rounded-md border bg-muted/40 px-3 py-2 font-mono text-xs">
                      {selected.bundle_hash}
                    </code>
                    <Button
                      variant="outline"
                      size="sm"
                      className="shrink-0"
                      onClick={() =>
                        copyFingerprint(
                          selected.bundle_hash,
                          t("skillsFingerprintCopied"),
                          t("skillsFingerprintCopyFailed"),
                        )
                      }
                    >
                      <CopyIcon />
                      {t("skillsCopy")}
                    </Button>
                  </div>
                </DetailSection>
                <section className="border-t pt-5">
                  <SkillContextHealthPanel
                    embedded
                    skillId={selected.id}
                    provider={provider}
                  />
                </section>
                <DetailSection title={t("skillsHealth")}>
                  <SkillHealthDetails skillId={selected.id} />
                </DetailSection>
                {detail && Object.keys(detail.frontmatter).length > 0 ? (
                  <DetailSection title={t("skillsMetadata")}>
                    <div className="grid gap-2 text-xs">
                      {Object.entries(detail.frontmatter).map(([key, value]) => (
                        <div key={key} className="grid gap-0.5 sm:grid-cols-[minmax(0,8rem)_1fr] sm:gap-3">
                          <span className="text-muted-foreground font-medium">{key}</span>
                          <span className="break-words">{value}</span>
                        </div>
                      ))}
                    </div>
                  </DetailSection>
                ) : null}
              </ScrollPane>
            </DialogContent>
          </Dialog>
          </div>
        </div>
        <p className="text-muted-foreground line-clamp-2 h-[2lh] text-sm leading-normal">
          {normalizeSkillDescription(selected.description) || t("skillsNoDescription")}
        </p>
      </div>
      <div className="flex min-h-0 flex-1 flex-col gap-3">
        <div
          className="flex shrink-0 flex-wrap gap-1 border-b"
          role="tablist"
          aria-label={t("skillsDetails")}
        >
          {(
            [
              ["bundle", t("skillsBundleFiles")],
              ["coverage", t("skillsCoverageConflicts")],
              ["installations", t("skillsInstallations")],
              ["consolidate", t("skillsConsolidate")],
            ] as Array<[DetailTab, string]>
          ).map(([value, label]) => (
            <button
              key={value}
              type="button"
              role="tab"
              aria-selected={tab === value}
              onClick={() => setTab(value)}
              className={cn(
                "border-b-2 px-3 py-2 text-sm font-medium transition-colors",
                tab === value
                  ? "border-primary text-foreground"
                  : "border-transparent text-muted-foreground",
              )}
            >
              {label}
            </button>
          ))}
        </div>

        {tab === "bundle" ? (
          <div className="flex min-h-0 flex-1 flex-col gap-3">
            <div className="flex min-h-0 flex-1 flex-col">
              {treeLoading ? (
                <div className="flex flex-1 items-center justify-center">
                  <Spinner />
                </div>
              ) : (
                <SkillBundlePanel
                  skillId={selected.source_id}
                  usedBy={sourceUsedBy}
                  assets={assets}
                  previewPath={previewPath}
                  onPreviewPathChange={onPreviewPathChange}
                  preview={preview}
                  previewLoading={previewLoading}
                />
              )}
            </div>
          </div>
        ) : null}

        {tab === "coverage" ? (
          <div className="flex min-h-0 flex-1 flex-col">
            <SkillCoverageConflictsPanel embedded skillId={selected.id} />
          </div>
        ) : null}

        {tab === "installations" ? (
          <div className="flex min-h-0 flex-1 flex-col">
            <ScrollPane className="min-h-0 flex-1" innerClassName="flex flex-col gap-3">
              {AGENTS.map((agent) => {
                const installations = selected.installations.filter(
                  (item) => item.used_by === agent && item.status === "active",
                );
                return (
                  <AgentInstallationSection
                    key={agent}
                    agent={agent}
                    installationTargets={selected.installation_targets}
                    currentWorkspace={currentWorkspace}
                    pending={pending}
                    pendingTarget={pendingTarget}
                    sourceUsedBy={sourceUsedBy}
                    t={t}
                    onInstall={onInstall}
                    onRemove={onRemove}
                  />
                );
              })}
            </ScrollPane>
          </div>
        ) : null}

        {tab === "consolidate" ? (
          <SkillConsolidatePanel
            active
            sourceId={selected.source_id}
            skillName={selected.name}
            catalogRealPath={realPathOf(selected) ?? null}
            pending={pending}
            onConfirm={onConsolidate}
            onDeleteInstallation={onDeleteInstallation}
            onRemoveSymlinks={onRemoveSymlinks}
          />
        ) : null}
      </div>
    </div>
  );
}
