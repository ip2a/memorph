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
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { SkillBundlePanel } from "@/features/skills/skill-bundle-panel";
import { SkillContextHealthPanel } from "@/features/skills/skill-context-health-panel";
import { SkillCoverageConflictsPanel } from "@/features/skills/skill-coverage-conflicts-panel";
import { SkillHealthDetails } from "@/features/skills/skill-health-tags";
import { formatBytes } from "@/lib/format";
import { useI18n } from "@/lib/i18n-context";
import type { SkillAsset, SkillCatalogItem, SkillDetail, SkillFilePreview } from "@/lib/types";

const AGENTS = ["claude", "codex", "gemini", "opencode", "hermes"] as const;

type DetailTab = "bundle" | "coverage" | "installations";

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
  sourceProvider,
  onSourceProviderChange,
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
  sourceProvider?: string;
  onSourceProviderChange: (provider: string) => void;
  pending: boolean;
  mutationError: Error | null;
  provider?: string;
  onInstall: (provider: string) => void;
  onRemove: (provider: string) => void;
}) {
  const { t } = useI18n();
  const [tab, setTab] = useState<DetailTab>("bundle");
  const [detailsOpen, setDetailsOpen] = useState(false);
  const selectedSource = selected?.installations.some(
    (item) => item.provider_id === sourceProvider,
  )
    ? sourceProvider
    : selected?.installations[0]?.provider_id;

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
      <Tabs
        value={tab}
        onValueChange={(value) => setTab(value as DetailTab)}
        className="flex min-h-0 flex-1 flex-col gap-3"
      >
        <TabsList className="grid w-full shrink-0 grid-cols-3">
          <TabsTrigger value="bundle">{t("skillsBundleFiles")}</TabsTrigger>
          <TabsTrigger value="coverage">{t("skillsCoverageConflicts")}</TabsTrigger>
          <TabsTrigger value="installations">{t("skillsInstallations")}</TabsTrigger>
        </TabsList>

        <TabsContent
          value="bundle"
          className="mt-0 flex min-h-0 flex-1 flex-col gap-3"
        >
          {selected.installations.length > 1 ? (
            <label className="flex shrink-0 items-center gap-2 text-sm">
              {t("skillsPreviewSource")}
              <select
                value={selectedSource}
                onChange={(event) => onSourceProviderChange(event.target.value)}
                className="h-8 rounded border bg-background px-2"
              >
                {selected.installations.map((item) => (
                  <option
                    key={`${item.provider_id}:${item.install_path}`}
                    value={item.provider_id}
                  >
                    {item.provider_id} · {item.scope_kind}
                  </option>
                ))}
              </select>
            </label>
          ) : null}
          <div className="flex min-h-0 flex-1 flex-col">
            {treeLoading ? (
              <div className="flex flex-1 items-center justify-center">
                <Spinner />
              </div>
            ) : (
              <SkillBundlePanel
                assets={assets}
                previewPath={previewPath}
                onPreviewPathChange={onPreviewPathChange}
                preview={preview}
                previewLoading={previewLoading}
              />
            )}
          </div>
        </TabsContent>

        <TabsContent
          value="coverage"
          className="mt-0 flex min-h-0 flex-1 flex-col"
        >
          <SkillCoverageConflictsPanel embedded skillId={selected.id} />
        </TabsContent>

        <TabsContent
          value="installations"
          className="mt-0 flex min-h-0 flex-1 flex-col"
        >
          <ScrollPane className="min-h-0 flex-1" innerClassName="flex flex-col gap-3">
            {AGENTS.map((agent) => {
              const installation = selected.installations.find(
                (item) => item.provider_id === agent && item.status === "active",
              );
              const managed =
                installation?.install_kind === "symlink" ||
                installation?.install_kind === "managed-copy";
              return (
                <div
                  key={agent}
                  className="flex items-center justify-between gap-3 rounded-lg border p-3"
                >
                  <div className="min-w-0">
                    <div className="flex flex-wrap gap-2">
                      <strong>{agent}</strong>
                      <Badge variant={installation ? "secondary" : "outline"}>
                        {installation ? t("installed") : t("notInstalled")}
                      </Badge>
                      {installation ? (
                        <Badge variant="outline">{installation.scope_kind}</Badge>
                      ) : null}
                    </div>
                    <p className="text-muted-foreground mt-1 truncate font-mono text-xs">
                      {installation?.install_path ?? "—"}
                    </p>
                  </div>
                  {installation ? (
                    <Button
                      variant="destructive"
                      size="sm"
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
                      disabled={pending || !selectedSource}
                      onClick={() => onInstall(agent)}
                    >
                      {t("install")}
                    </Button>
                  )}
                </div>
              );
            })}
          </ScrollPane>
        </TabsContent>
      </Tabs>
    </div>
  );
}
