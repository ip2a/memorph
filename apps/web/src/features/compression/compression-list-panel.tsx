import { useMemo } from "react";
import { Link } from "react-router-dom";
import { FileArchiveIcon } from "lucide-react";
import { EntityRow } from "@/components/shared/entity-row";
import { PageEmpty, PageError, PageSkeleton } from "@/components/shared/page-states";
import { ProviderLogo } from "@/components/shared/provider-logo";
import { Button } from "@/components/ui/button";
import { useCompressionArchives } from "@/features/compression/queries";
import { useManagerMeta } from "@/features/manager/queries";
import { formatBytes, formatDateTime } from "@/lib/format";
import { useI18n } from "@/lib/i18n-context";
import type { CompressionArchiveSummary } from "@/lib/types";

type CompressedSessionEntry = {
  key: string;
  providerId: string;
  sessionId: string;
  archives: CompressionArchiveSummary[];
  latestCreatedAt: string;
  totalStoredBytes: number;
  workspaceDir?: string | null;
};

function sessionKey(providerId: string, sessionId: string) {
  return `${providerId}:${sessionId}`;
}

function groupArchivesBySession(archives: CompressionArchiveSummary[]): CompressedSessionEntry[] {
  const map = new Map<string, CompressedSessionEntry>();
  for (const archive of archives) {
    const key = sessionKey(archive.source_provider_id, archive.canonical_id);
    const existing = map.get(key);
    if (existing) {
      existing.archives.push(archive);
      existing.totalStoredBytes += archive.stored_size_bytes;
      if (archive.created_at > existing.latestCreatedAt) {
        existing.latestCreatedAt = archive.created_at;
      }
      if (!existing.workspaceDir && archive.workspace_dir) {
        existing.workspaceDir = archive.workspace_dir;
      }
    } else {
      map.set(key, {
        key,
        providerId: archive.source_provider_id,
        sessionId: archive.canonical_id,
        archives: [archive],
        latestCreatedAt: archive.created_at,
        totalStoredBytes: archive.stored_size_bytes,
        workspaceDir: archive.workspace_dir,
      });
    }
  }
  return [...map.values()]
    .map((entry) => ({
      ...entry,
      archives: [...entry.archives].sort((left, right) => right.created_at.localeCompare(left.created_at)),
    }))
    .sort((left, right) => right.latestCreatedAt.localeCompare(left.latestCreatedAt));
}

export function CompressionListPanel() {
  const { t } = useI18n();
  const meta = useManagerMeta();
  const workspace = meta.data?.selected_workspace ?? undefined;
  const archives = useCompressionArchives({ workspace, limit: 200 });

  const sessions = useMemo(
    () => groupArchivesBySession(archives.data ?? []),
    [archives.data],
  );

  if (archives.isLoading || meta.isLoading) return <PageSkeleton />;
  if (archives.error) {
    return (
      <PageError
        title={t("compressionArchivesLoadFailed")}
        message={archives.error.message}
      />
    );
  }
  if (meta.error) {
    return (
      <PageError
        title={t("compressionWorkspaceLoadFailed")}
        message={meta.error.message}
      />
    );
  }

  if (!sessions.length) {
    return (
      <PageEmpty
        title={t("compressionNoCompressedSessions")}
        description={t("compressionNoCompressedSessionsDescription")}
      />
    );
  }

  return (
    <div className="flex flex-col gap-2" data-compression-list-panel>
      {sessions.map((entry) => {
        const href = `/compression?session=${encodeURIComponent(entry.key)}`;
        return (
          <EntityRow
            key={entry.key}
            data-compression-session-row
            actions={
              <Button asChild variant="outline" size="sm">
                <Link to={href}>{t("view")}</Link>
              </Button>
            }
          >
            <div className="flex min-w-0 flex-col gap-1.5">
              <Link to={href} className="truncate text-sm font-medium hover:underline">
                {entry.sessionId}
              </Link>
              <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
                <span className="inline-flex items-center gap-1.5">
                  <ProviderLogo providerId={entry.providerId} size="xs" alt={entry.providerId} />
                  <span>{entry.providerId}</span>
                </span>
                <span className="inline-flex items-center gap-1">
                  <FileArchiveIcon className="size-3" aria-hidden />
                  {t("compressionArchiveCount", { count: entry.archives.length })}
                </span>
                <span>{formatBytes(entry.totalStoredBytes)}</span>
                <span>{formatDateTime(entry.latestCreatedAt)}</span>
              </div>
            </div>
          </EntityRow>
        );
      })}
    </div>
  );
}