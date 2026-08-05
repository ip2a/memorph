import type { ReadinessPhaseName, ReadinessState } from "@/lib/types";
import type { I18nKey } from "@/lib/i18n-core";

export const READINESS_PHASE_ORDER: ReadinessPhaseName[] = [
  "foundation",
  "agents",
  "sessions",
  "session_stats",
  "skills",
  "usage",
  "derived",
];

export const READINESS_PHASE_LABEL_KEYS: Record<ReadinessPhaseName, I18nKey> = {
  foundation: "readinessPhaseFoundation",
  agents: "readinessPhaseAgents",
  sessions: "readinessPhaseSessions",
  session_stats: "readinessPhaseSessionStats",
  skills: "readinessPhaseSkills",
  usage: "readinessPhaseUsage",
  derived: "readinessPhaseDerived",
};

export const READINESS_STATE_LABEL_KEYS: Record<ReadinessState, I18nKey> = {
  ready: "readinessReady",
  partial: "readinessPartial",
  degraded: "readinessWarning",
  error: "readinessError",
};

export function readinessNeedsRebuild(state: ReadinessState | undefined) {
  return state !== "ready";
}
