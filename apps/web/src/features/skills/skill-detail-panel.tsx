import { CopyIcon, Trash2Icon } from "lucide-react";
import { toast } from "sonner";
import { PageError } from "@/components/shared/page-states";
import { ScrollPane } from "@/components/shared/scroll-pane";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Empty,
  EmptyHeader,
  EmptyTitle,
} from "@/components/ui/empty";
import { Spinner } from "@/components/ui/spinner";
import { SkillBundlePanel } from "@/features/skills/skill-bundle-panel";
import { SkillContextHealthPanel } from "@/features/skills/skill-context-health-panel";
import { SkillCoverageConflictsPanel } from "@/features/skills/skill-coverage-conflicts-panel";
import { formatBytes } from "@/lib/format";
import { useI18n } from "@/lib/i18n-context";
import type { SkillAsset, SkillCatalogItem, SkillDetail, SkillFilePreview } from "@/lib/types";

const AGENTS = ["claude", "codex", "gemini", "opencode", "hermes"] as const;

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
  const selectedSource = selected?.installations.some(
    (item) => item.provider_id === sourceProvider,
  )
    ? sourceProvider
    : selected?.installations[0]?.provider_id;

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
    <>
      {mutationError ? (
        <PageError title={t("skillsActionFailed")} message={mutationError.message} />
      ) : null}
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <h2 className="text-xl font-semibold">{selected.name}</h2>
          <p className="text-muted-foreground mt-1 text-sm">
            {selected.description || t("skillsNoDescription")}
          </p>
        </div>
        <Button
          variant="outline"
          size="sm"
          onClick={() => copyFingerprint(selected.bundle_hash, t("skillsFingerprintCopied"), t("skillsFingerprintCopyFailed"))}
        >
          <CopyIcon />
          {t("skillsFingerprint")}
        </Button>
      </div>
      <ScrollPane className="flex-1" innerClassName="flex flex-col gap-5">
        <div className="flex flex-wrap gap-2 text-xs">
          <Badge variant="outline">{t("skillsFilesCount", { count: selected.file_count })}</Badge>
          <Badge variant="outline">{formatBytes(selected.total_bytes)}</Badge>
          {selected.version ? (
            <Badge variant="outline">v{selected.version}</Badge>
          ) : null}
          {selected.missing ? <Badge variant="destructive">{t("skillsMissing")}</Badge> : null}
        </div>
        {selected.installations.length > 1 ? (
          <label className="flex items-center gap-2 text-sm">
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
        <section className="flex flex-col gap-2">
          <h3 className="text-sm font-semibold">{t("skillsBundleFiles")}</h3>
          {treeLoading ? (
            <Spinner />
          ) : (
            <SkillBundlePanel
              skillId={selected.source_id}
              assets={assets}
              previewPath={previewPath}
              onPreviewPathChange={onPreviewPathChange}
              preview={preview}
              previewLoading={previewLoading}
            />
          )}
        </section>
        <section className="flex flex-col gap-3 border-t pt-5">
          <h3 className="text-sm font-semibold">{t("skillsBudgetHealth")}</h3>
          <SkillContextHealthPanel skillId={selected.id} provider={provider} />
        </section>
        <section className="flex flex-col gap-3 border-t pt-5">
          <h3 className="text-sm font-semibold">{t("skillsCoverageConflicts")}</h3>
          <SkillCoverageConflictsPanel embedded skillId={selected.id} />
        </section>
        <section className="flex flex-col gap-3 border-t pt-5">
          <h3 className="text-sm font-semibold">{t("skillsInstallations")}</h3>
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
                  <div className="flex gap-2">
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
        </section>
        {detail ? (
          <section className="rounded-lg border p-3">
            <h3 className="text-sm font-semibold">{t("skillsMetadata")}</h3>
            <div className="mt-2 grid gap-1 text-xs">
              {Object.entries(detail.frontmatter).map(([key, value]) => (
                <div key={key}>
                  <span className="text-muted-foreground">{key}:</span> {value}
                </div>
              ))}
            </div>
          </section>
        ) : null}
      </ScrollPane>
    </>
  );
}
