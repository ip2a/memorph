import { RefreshCw } from "lucide-react";
import { useI18n } from "@/lib/i18n-context";
import { useReadiness } from "@/features/readiness/queries";

export function ReadinessIndicator() {
  const { t } = useI18n();
  const { effectiveReadiness, isRunning, isReconciling } = useReadiness();
  const state = effectiveReadiness?.state;
  const busy = isRunning || isReconciling;

  if (!busy) return null;
  if (!effectiveReadiness && !isReconciling) return null;
  if (state === "ready") return null;

  return (
    <div className="flex min-w-0 items-center gap-2">
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
