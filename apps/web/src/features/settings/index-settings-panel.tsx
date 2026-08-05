import { AlertTriangle, RefreshCw } from "lucide-react";
import type { ReactNode } from "react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";
import { manualReconcilePayload, useReadiness } from "@/features/readiness/queries";
import {
  READINESS_PHASE_LABEL_KEYS,
  READINESS_PHASE_ORDER,
  READINESS_STATE_LABEL_KEYS,
  readinessNeedsRebuild,
} from "@/features/readiness/readiness-phases";
import { useI18n } from "@/lib/i18n-context";
import { cn } from "@/lib/utils";
import type { ReadinessPhase, ReadinessState } from "@/lib/types";

function readinessStateVariant(state: ReadinessState | undefined) {
  if (state === "ready") return "secondary";
  if (state === "partial") return "outline";
  return "destructive";
}

function phaseStateVariant(state: ReadinessPhase["state"]) {
  if (state === "ready") return "secondary";
  if (state === "partial") return "outline";
  return "destructive";
}

export function IndexSettingsPanel() {
  const { t } = useI18n();
  const { workspace, effectiveReadiness, isRunning, isReconciling, reconcile, readiness } = useReadiness();
  const state = effectiveReadiness?.state;
  const busy = isRunning || isReconciling;
  const needsRebuild = readinessNeedsRebuild(state);

  if (readiness.isLoading) {
    return (
      <div className="flex items-center gap-2 text-sm text-muted-foreground">
        <Spinner />
        {t("loadingSettings")}
      </div>
    );
  }

  return (
    <section className="flex flex-col gap-4" data-settings-section="index">
      <div className="border-b pb-2">
        <h3 className="text-base font-semibold">{t("indexSection")}</h3>
        <p className="mt-1 text-sm text-muted-foreground">{t("indexSectionDescription")}</p>
      </div>

      <div className="divide-y">
        <IndexSettingsRow title={t("readinessOverallStatus")}>
          <Badge variant={readinessStateVariant(state)}>{t(READINESS_STATE_LABEL_KEYS[state ?? "partial"])}</Badge>
        </IndexSettingsRow>
        <IndexSettingsRow title={t("readinessRebuildStatus")}>
          <span className={cn("text-sm", needsRebuild ? "text-destructive" : "text-muted-foreground")}>
            {needsRebuild ? t("readinessRebuildNeeded") : t("readinessRebuildNotNeeded")}
          </span>
        </IndexSettingsRow>
        {workspace ? (
          <IndexSettingsRow title={t("workspace")}>
            <span className="max-w-md truncate font-mono text-xs text-muted-foreground">{workspace}</span>
          </IndexSettingsRow>
        ) : null}
      </div>

      {effectiveReadiness?.phases ? (
        <div className="flex flex-col gap-2">
          <h4 className="text-sm font-medium">{t("readinessPhases")}</h4>
          <div className="divide-y rounded-md border">
            {READINESS_PHASE_ORDER.map((phase) => {
              const phaseState = effectiveReadiness.phases[phase];
              const labelKey = READINESS_PHASE_LABEL_KEYS[phase];
              return (
                <div key={phase} className="flex min-h-10 items-start gap-3 px-3 py-2.5">
                  <div className="min-w-0 flex-1">
                    <div className="text-sm font-medium">{t(labelKey)}</div>
                    {phaseState.message ? (
                      <div className="mt-0.5 text-xs text-muted-foreground">{phaseState.message}</div>
                    ) : null}
                  </div>
                  <Badge variant={phaseStateVariant(phaseState.state)} className="shrink-0">
                    {t(READINESS_STATE_LABEL_KEYS[phaseState.state])}
                  </Badge>
                </div>
              );
            })}
          </div>
        </div>
      ) : null}

      <div className="flex flex-col gap-2 sm:flex-row sm:items-center">
        <Button
          type="button"
          variant={needsRebuild ? "destructive" : "outline"}
          disabled={busy}
          onClick={() => reconcile(manualReconcilePayload(workspace))}
        >
          {busy ? <RefreshCw data-icon="inline-start" className="animate-spin" /> : needsRebuild ? <AlertTriangle data-icon="inline-start" /> : null}
          {busy ? t("readinessRunning") : t("readinessAction")}
        </Button>
        <p className="text-sm text-muted-foreground">{t("readinessActionDescription")}</p>
      </div>
    </section>
  );
}

function IndexSettingsRow({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div className="flex min-h-10 items-center gap-4 py-3">
      <div className="min-w-0 flex-1 text-sm font-medium">{title}</div>
      <div className="flex shrink-0 items-center justify-end">{children}</div>
    </div>
  );
}
