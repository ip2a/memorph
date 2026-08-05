import { useEffect, useState } from "react";
import { CopyIcon, InfoIcon, Trash2Icon } from "lucide-react";
import { toast } from "sonner";
import { PageError } from "@/components/shared/page-states";
import { ScrollPane } from "@/components/shared/scroll-pane";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
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
import { SkillContextHealthPanel } from "@/features/skills/skill-context-health-panel";
import { SkillCoverageConflictsPanel } from "@/features/skills/skill-coverage-conflicts-panel";
import { SkillHealthDetails } from "@/features/skills/skill-health-tags";
import { formatBytes } from "@/lib/format";
import { useI18n } from "@/lib/i18n-context";
import { cn } from "@/lib/utils";
import type { I18nKey } from "@/lib/i18n-core";
import type { SkillAsset, SkillCatalogItem, SkillDetail, SkillFilePreview } from "@/lib/types";

const AGENTS = ["claude", "codex", "gemini", "opencode", "hermes"] as const;

type DetailTab = "bundle" | "coverage" | "installations";

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

function InstallationRow({
  agent,
  installation,
  pending,
  sourceUsedBy,
  t,
  onInstall,
  onRemove,
}: {
  agent: (typeof AGENTS)[number];
  installation?: SkillCatalogItem["installations"][number];
  pending: boolean;
  sourceUsedBy?: string;
  t: (key: I18nKey, vars?: Record<string, string | number>) => string;
  onInstall: (agent: string) => void;
  onRemove: (agent: string) => void;
}) {
  const managed =
    installation?.install_kind === "symlink" ||
    installation?.install_kind === "managed-copy";
  const linkBroken =
    installation?.install_kind === "symlink" &&
    installation.link_status === "broken";

  return (
    <div className="flex items-start justify-between gap-3 rounded-lg border p-3">
      <div className="min-w-0 flex-1 space-y-2">
        <div className="flex flex-wrap items-center gap-2">
          <strong>{agent}</strong>
          <Badge variant={installation ? "secondary" : "outline"}>
            {installation ? t("installed") : t("notInstalled")}
          </Badge>
          {installation ? (
            <>
              <Badge variant="outline">{installation.scope_kind}</Badge>
              <Badge variant="outline">
                {installKindLabel(installation.install_kind, t)}
              </Badge>
              {linkBroken ? (
                <Badge variant="destructive">{t("skillsLinkBroken")}</Badge>
              ) : null}
            </>
          ) : null}
        </div>
        {installation ? (
          <div className="space-y-1 text-xs">
            <div>
              <span className="text-muted-foreground">{t("skillsInstallPath")}</span>
              <p className="break-all font-mono">{installation.install_path}</p>
            </div>
            {installation.scope_kind === "project" && installation.workspace_dir ? (
              <div>
                <span className="text-muted-foreground">
                  {t("skillsProjectScope")}
                </span>
                <p className="break-all font-mono">{installation.workspace_dir}</p>
              </div>
            ) : null}
            {installation.install_kind === "symlink" && installation.symlink_target ? (
              <div>
                <span className="text-muted-foreground">
                  {t("skillsSymlinkTarget")}
                </span>
                <p className="break-all font-mono">{installation.symlink_target}</p>
              </div>
            ) : null}
          </div>
        ) : (
          <p className="text-muted-foreground text-xs">—</p>
        )}
      </div>
      {installation ? (
        <Button
          variant="destructive"
          size="sm"
          className="shrink-0"
          disabled={pending || !managed}
          title={!managed ? t("skillsUserOwnedHint") : undefined}
          onClick={() => onRemove(agent)}
        >
          <Trash2Icon />
          {t("remove")}
        </Button>
      ) : (
        <Button
          size="sm"
          className="shrink-0"
          disabled={pending || !sourceUsedBy}
          onClick={() => onInstall(agent)}
        >
          {t("install")}
        </Button>
      )}
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
  mutationError,
  provider,
  onInstall,
  onRemove,
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
  mutationError: Error | null;
  provider?: string;
  onInstall: (agent: string) => void;
  onRemove: (agent: string) => void;
}) {
  const { t } = useI18n();
  const [tab, setTab] = useState<DetailTab>("bundle");
  const [detailsOpen, setDetailsOpen] = useState(false);
  const sourceUsedBy = selected?.installations.find(
    (item) => item.status === "active",
  )?.used_by;

  useEffect(() => {
    setTab("bundle");
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
          <Dialog open={detailsOpen} onOpenChange={setDetailsOpen}>
            <DialogTrigger asChild>
              <Button variant="outline" size="sm" className="shrink-0">
                <InfoIcon data-icon="inline-start" />
                {t("skillsDetails")}
              </Button>
            </DialogTrigger>
            <DialogContent className="flex h-[min(85dvh,840px)] w-[min(960px,calc(100vw-2rem))] max-w-[calc(100vw-2rem)] flex-col gap-0 overflow-hidden p-0 sm:max-w-3xl">
              <DialogHeader className="shrink-0 border-b px-6 py-4">
                <DialogTitle>{t("skillsDetails")}</DialogTitle>
              </DialogHeader>
              <ScrollPane className="min-h-0 flex-1" innerClassName="space-y-6 px-6 py-4 text-sm">
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
                    <Badge key={tag} variant="outline">{tag}</Badge>
                  ))}
                </div>
                <div className="space-y-2 border-t pt-5">
                  <h3 className="text-sm font-semibold">{t("skillsUsedBy")}</h3>
                  {(detail?.used_by ?? selected.used_by).length ? (
                    <div className="flex flex-wrap gap-2">
                      {(detail?.used_by ?? selected.used_by).map((agent) => (
                        <Badge key={agent} variant="secondary">{agent}</Badge>
                      ))}
                    </div>
                  ) : (
                    <p className="text-muted-foreground text-sm">{t("skillsNoUsedBy")}</p>
                  )}
                </div>
                <div className="space-y-2">
                  <p className="text-muted-foreground text-xs">{t("skillsFingerprint")}</p>
                  <div className="flex items-center gap-2">
                    <code className="min-w-0 flex-1 break-all rounded border bg-muted/40 px-3 py-2 font-mono text-xs">
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
                </div>
                <div className="space-y-3 border-t pt-5">
                  <SkillContextHealthPanel
                    embedded
                    skillId={selected.id}
                    provider={provider}
                  />
                </div>
                <div className="space-y-2 border-t pt-5">
                  <h3 className="text-sm font-semibold">{t("skillsHealth")}</h3>
                  <SkillHealthDetails skillId={selected.id} />
                </div>
                {detail && Object.keys(detail.frontmatter).length > 0 ? (
                  <div className="space-y-2 border-t pt-5">
                    <h3 className="text-sm font-semibold">{t("skillsMetadata")}</h3>
                    <div className="grid gap-1 text-xs">
                      {Object.entries(detail.frontmatter).map(([key, value]) => (
                        <div key={key}>
                          <span className="text-muted-foreground">{key}:</span> {value}
                        </div>
                      ))}
                    </div>
                  </div>
                ) : null}
              </ScrollPane>
            </DialogContent>
          </Dialog>
        </div>
        <p className="text-muted-foreground line-clamp-2 h-[2lh] text-sm leading-normal">
          {selected.description || t("skillsNoDescription")}
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
              {AGENTS.flatMap((agent) => {
                const installations = selected.installations.filter(
                  (item) => item.used_by === agent && item.status === "active",
                );
                if (!installations.length) {
                  return [
                    <InstallationRow
                      key={agent}
                      agent={agent}
                      pending={pending}
                      sourceUsedBy={sourceUsedBy}
                      t={t}
                      onInstall={onInstall}
                      onRemove={onRemove}
                    />,
                  ];
                }
                return installations.map((installation) => (
                  <InstallationRow
                    key={`${agent}:${installation.install_path}`}
                    agent={agent}
                    installation={installation}
                    pending={pending}
                    sourceUsedBy={sourceUsedBy}
                    t={t}
                    onInstall={onInstall}
                    onRemove={onRemove}
                  />
                ));
              })}
            </ScrollPane>
          </div>
        ) : null}
      </div>
    </div>
  );
}
