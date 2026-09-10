import { AlertTriangle, RefreshCw } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Field, FieldContent, FieldDescription, FieldGroup, FieldTitle } from "@/components/ui/field";
import { Spinner } from "@/components/ui/spinner";
import { manualReconcilePayload, useReadiness } from "@/features/readiness/queries";
import {
  READINESS_PHASE_LABEL_KEYS,
  READINESS_PHASE_ORDER,
  READINESS_STATE_LABEL_KEYS,
  readinessNeedsRebuild,
} from "@/features/readiness/readiness-phases";
import { useI18n } from "@/lib/i18n-context";
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
      </div>

      <FieldGroup>
        <Field orientation="responsive">
          <FieldContent>
            <FieldTitle>{t("readinessOverallStatus")}</FieldTitle>
          </FieldContent>
          <Badge variant={readinessStateVariant(state)}>{t(READINESS_STATE_LABEL_KEYS[state ?? "partial"])}</Badge>
        </Field>

        {workspace ? (
          <Field orientation="responsive">
            <FieldContent>
              <FieldTitle>{t("workspace")}</FieldTitle>
            </FieldContent>
            <span className="max-w-md truncate font-mono text-xs text-muted-foreground">{workspace}</span>
          </Field>
        ) : null}

        {effectiveReadiness?.phases
          ? READINESS_PHASE_ORDER.map((phase) => {
              const phaseState = effectiveReadiness.phases[phase];
              const labelKey = READINESS_PHASE_LABEL_KEYS[phase];
              return (
                <Field key={phase} orientation="responsive">
                  <FieldContent>
                    <FieldTitle>{t(labelKey)}</FieldTitle>
                    {phaseState.message ? <FieldDescription>{phaseState.message}</FieldDescription> : null}
                  </FieldContent>
                  <Badge variant={phaseStateVariant(phaseState.state)}>
                    {t(READINESS_STATE_LABEL_KEYS[phaseState.state])}
                  </Badge>
                </Field>
              );
            })
          : null}

        <Field orientation="responsive">
          <FieldContent>
            <FieldTitle>{t("readinessAction")}</FieldTitle>
            <FieldDescription>{t("readinessActionDescription")}</FieldDescription>
          </FieldContent>
          <Button
            type="button"
            variant={needsRebuild ? "destructive" : "outline"}
            disabled={busy}
            onClick={() => reconcile(manualReconcilePayload(workspace))}
          >
            {busy ? <RefreshCw data-icon="inline-start" className="animate-spin" /> : needsRebuild ? <AlertTriangle data-icon="inline-start" /> : null}
            {busy ? t("readinessRunning") : t("readinessAction")}
          </Button>
        </Field>
      </FieldGroup>
    </section>
  );
}
