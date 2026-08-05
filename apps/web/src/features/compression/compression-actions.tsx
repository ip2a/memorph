import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { Link } from "react-router-dom";
import { toast } from "sonner";
import { ArchiveIcon } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Spinner } from "@/components/ui/spinner";
import type { SessionActionTarget } from "@/features/sessions/session-action-target";
import { useApplyCompression } from "@/features/compression/queries";
import { formatBytes } from "@/lib/format";
import { queryKeys } from "@/lib/query-keys";
import { useI18n } from "@/lib/i18n-context";
import type { ActiveCompressionPolicy, ApplyCompressionResult } from "@/lib/types";

function compressionSummary(result: ApplyCompressionResult, t: ReturnType<typeof useI18n>["t"]) {
  const candidates = result.report?.candidates?.length ?? 0;
  const saved = result.report?.estimated_bytes_saved ?? 0;
  return t("compressionSummary", { count: candidates, size: formatBytes(saved) });
}

function compressionArchiveHref(archiveRef: string) {
  return `/compression?archive_ref=${encodeURIComponent(archiveRef)}`;
}

function isArchiveRef(line: string) {
  return line.startsWith("memorph-archive://");
}

export function CompressSessionDialog({
  target,
  open,
  onOpenChange,
}: {
  target: SessionActionTarget | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const { t } = useI18n();
  const queryClient = useQueryClient();
  const targetKey = target ? `${target.providerId}:${target.sessionId}` : "";
  const [resultState, setResultState] = useState<{ key: string; result: ApplyCompressionResult } | null>(null);
  const [policy, setPolicy] = useState<ActiveCompressionPolicy>({
    protect_recent_message_events: 6,
    min_candidate_bytes: 4096,
    min_savings_ratio_percent: 20,
    mode: "auto",
  });
  const compressMutation = useApplyCompression();
  const result = resultState?.key === targetKey ? resultState.result : null;
  const primaryArchiveRef = result?.archive_refs?.[0] ?? "";

  function handleOpenChange(nextOpen: boolean) {
    if (!nextOpen) setResultState(null);
    onOpenChange(nextOpen);
  }

  function runCompression() {
    if (!target) throw new Error("Missing session target");
    compressMutation.mutate(
      {
        source_provider_id: target.providerId,
        target_provider_id: target.providerId,
        session_id: target.sessionId,
        policy,
      },
      {
        onSuccess: async (nextResult) => {
          await Promise.all([
            queryClient.invalidateQueries({ queryKey: queryKeys.home }),
            queryClient.invalidateQueries({ queryKey: queryKeys.sessionsRoot }),
            target ? queryClient.invalidateQueries({ queryKey: queryKeys.session(target.providerId, target.sessionId) }) : Promise.resolve(),
            queryClient.invalidateQueries({ queryKey: ["compression"] }),
            queryClient.invalidateQueries({ queryKey: queryKeys.compressionProviders }),
          ]);
          setResultState({ key: targetKey, result: nextResult });
          toast.success(t("compression"), { description: compressionSummary(nextResult, t) });
        },
      },
    );
  }

  const lines = result ? [...(result.archive_refs ?? []), ...(result.files ?? [])] : [];

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogContent className="sm:max-w-lg" data-compress-session-dialog>
        <DialogHeader>
          <DialogTitle>{result ? t("compressionComplete") : t("compressionSession")}</DialogTitle>
          <DialogDescription>
            {result ? compressionSummary(result, t) : t("compressionConfirmation")}
          </DialogDescription>
        </DialogHeader>
        {result ? (
          <div className="flex flex-col gap-3">
            <div className="rounded-md border p-3">
              <div className="font-medium">{target?.title || target?.sessionId || t("compressionSessionFallback")}</div>
              <div className="break-all font-mono text-xs text-muted-foreground">
                {target?.providerId || "-"} / {target?.sessionId || "-"}
              </div>
            </div>
            <div className="grid grid-cols-2 gap-2 text-sm">
              <div className="rounded-md border p-2"><span className="text-muted-foreground">{t("compressionSourceBefore")}</span><div>{formatBytes(result.source_bytes_before)}</div></div>
              <div className="rounded-md border p-2"><span className="text-muted-foreground">{t("compressionSourceAfter")}</span><div>{formatBytes(result.source_bytes_after)}</div></div>
            </div>
            <div className="flex max-h-48 flex-col gap-2 overflow-auto rounded-md border p-3" data-compression-result-lines>
              {lines.length ? (
                lines.map((line) => isArchiveRef(line) ? (
                  <Link key={line} to={compressionArchiveHref(line)} className="break-all font-mono text-xs underline-offset-4 hover:underline">
                    {line}
                  </Link>
                ) : (
                  <div key={line} className="break-all font-mono text-xs">{line}</div>
                ))
              ) : (
                <div className="text-sm text-muted-foreground">{t("compressionNoResultFiles")}</div>
              )}
            </div>
          </div>
        ) : (
          <div className="flex flex-col gap-3">
            <input type="hidden" name="provider" value={target?.providerId || ""} />
            <input type="hidden" name="session_id" value={target?.sessionId || ""} />
            <p className="text-sm text-muted-foreground">{t("compressionPolicyDescription")}</p>
            <div className="grid grid-cols-3 gap-3">
              <label className="grid gap-1 text-xs text-muted-foreground">
                {t("compressionRecentMessagesKept")}
                <input className="h-9 rounded-md border bg-background px-2 text-foreground" type="number" min={0} value={policy.protect_recent_message_events} onChange={(event) => setPolicy({ ...policy, protect_recent_message_events: Math.max(0, Number(event.target.value) || 0) })} />
              </label>
              <label className="grid gap-1 text-xs text-muted-foreground">
                {t("compressionMinimumBytes")}
                <input className="h-9 rounded-md border bg-background px-2 text-foreground" type="number" min={1} value={policy.min_candidate_bytes} onChange={(event) => setPolicy({ ...policy, min_candidate_bytes: Math.max(1, Number(event.target.value) || 1) })} />
              </label>
              <label className="grid gap-1 text-xs text-muted-foreground">
                {t("compressionMinimumSavings")}
                <input className="h-9 rounded-md border bg-background px-2 text-foreground" type="number" min={0} max={100} value={policy.min_savings_ratio_percent} onChange={(event) => setPolicy({ ...policy, min_savings_ratio_percent: Math.min(100, Math.max(0, Number(event.target.value) || 0)) })} />
              </label>
            </div>
            <div className="break-all rounded-md border p-3 font-mono text-xs" data-compression-path-line>
              {target?.providerId || "-"} / {target?.sessionId || "-"}
            </div>
          </div>
        )}
        <DialogFooter>
          {result && primaryArchiveRef ? (
            <Button asChild variant="outline">
              <Link to={compressionArchiveHref(primaryArchiveRef)}>
                <ArchiveIcon data-icon="inline-start" />
                {t("compressionOpenArchive")}
              </Link>
            </Button>
          ) : result && target ? (
            <Button asChild variant="outline">
              <Link to={`/sessions/${encodeURIComponent(target.providerId)}/${encodeURIComponent(target.sessionId)}`}>
                <ArchiveIcon data-icon="inline-start" />
                {t("compressionOpenSession")}
              </Link>
            </Button>
          ) : null}
          <Button type="button" variant="outline" onClick={() => handleOpenChange(false)} disabled={compressMutation.isPending}>
            {result ? t("compressionClose") : t("cancel")}
          </Button>
          {!result ? (
            <Button type="button" onClick={runCompression} disabled={!target || compressMutation.isPending}>
              {compressMutation.isPending ? <Spinner data-icon="inline-start" /> : null}
              {t("confirmAction")}
            </Button>
          ) : null}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
