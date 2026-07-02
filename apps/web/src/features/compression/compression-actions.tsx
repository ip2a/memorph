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
import type { ApplyCompressionResult } from "@/lib/types";

function compressionSummary(result: ApplyCompressionResult) {
  const candidates = result.report?.candidates?.length ?? 0;
  const saved = result.report?.estimated_bytes_saved ?? 0;
  return `${candidates} segments, ${formatBytes(saved)} saved`;
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
  const queryClient = useQueryClient();
  const targetKey = target ? `${target.providerId}:${target.sessionId}` : "";
  const [resultState, setResultState] = useState<{ key: string; result: ApplyCompressionResult } | null>(null);
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
        policy: {
          protect_recent_message_events: 6,
          min_candidate_bytes: 4096,
          min_savings_ratio_percent: 20,
          mode: "auto",
        },
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
          toast.success("Compression", { description: compressionSummary(nextResult) });
        },
      },
    );
  }

  const lines = result ? [...(result.archive_refs ?? []), ...(result.files ?? [])] : [];

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogContent className="sm:max-w-lg" data-compress-session-dialog>
        <DialogHeader>
          <DialogTitle>{result ? "Compression Complete" : "Compress Session"}</DialogTitle>
          <DialogDescription>
            {result ? compressionSummary(result) : "Confirm active compression for this provider session."}
          </DialogDescription>
        </DialogHeader>
        {result ? (
          <div className="flex flex-col gap-3">
            <div className="rounded-md border p-3">
              <div className="font-medium">{target?.title || target?.sessionId || "Session"}</div>
              <div className="break-all font-mono text-xs text-muted-foreground">
                {target?.providerId || "-"} / {target?.sessionId || "-"}
              </div>
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
                <div className="text-sm text-muted-foreground">No archive refs or files were returned.</div>
              )}
            </div>
          </div>
        ) : (
          <div className="flex flex-col gap-3">
            <input type="hidden" name="provider" value={target?.providerId || ""} />
            <input type="hidden" name="session_id" value={target?.sessionId || ""} />
            <p className="text-sm text-muted-foreground">This will create durable compression archives and update the session with compressed summaries.</p>
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
                Open Archive
              </Link>
            </Button>
          ) : result && target ? (
            <Button asChild variant="outline">
              <Link to={`/sessions/${encodeURIComponent(target.providerId)}/${encodeURIComponent(target.sessionId)}`}>
                <ArchiveIcon data-icon="inline-start" />
                Open Session
              </Link>
            </Button>
          ) : null}
          <Button type="button" variant="outline" onClick={() => handleOpenChange(false)} disabled={compressMutation.isPending}>
            {result ? "Close" : "Cancel"}
          </Button>
          {!result ? (
            <Button type="button" onClick={runCompression} disabled={!target || compressMutation.isPending}>
              {compressMutation.isPending ? <Spinner data-icon="inline-start" /> : null}
              Confirm
            </Button>
          ) : null}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
