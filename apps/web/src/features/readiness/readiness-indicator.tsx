import { useEffect } from "react";
import { RefreshCw } from "lucide-react";
import { useI18n } from "@/lib/i18n-context";
import { useReadiness } from "@/features/readiness/queries";
import {
  hasSeenReadinessFirstRun,
  markReadinessFirstRunSeen,
} from "@/features/readiness/readiness-first-run";

export function ReadinessIndicator() {
  const { t } = useI18n();
  const { workspace, effectiveReadiness, isRunning, isReconciling } = useReadiness();
  const state = effectiveReadiness?.state;
  const busy = isRunning || isReconciling;
  const firstRunPending = !hasSeenReadinessFirstRun(workspace);
  const showFirstRunNote = firstRunPending && busy;

  useEffect(() => {
    if (!workspace || !firstRunPending || !effectiveReadiness) return;
    if (state === "ready" || !busy) {
      markReadinessFirstRunSeen(workspace);
    }
  }, [busy, effectiveReadiness, firstRunPending, state, workspace]);

  if (!firstRunPending) return null;
  if (!effectiveReadiness && !isReconciling) return null;
  if (state === "ready") return null;

  return (
    <div className="flex min-w-0 items-center gap-2">
      {showFirstRunNote ? (
        <span
          className="hidden max-w-64 truncate text-xs text-muted-foreground xl:inline"
          title={t("readinessFirstRunDescription")}
        >
          {t("readinessFirstRunDescription")}
        </span>
      ) : null}
      <span
        className="inline-flex shrink-0 items-center gap-1.5 text-xs text-muted-foreground"
        title={t("readinessRunning")}
      >
        <RefreshCw className="size-3.5 animate-spin" />
        <span className="hidden sm:inline">{t("readinessRunning")}</span>
      </span>
    </div>
  );
}
