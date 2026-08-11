import { useEffect, useMemo, useRef, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { getMeta, getReadiness, getReadinessOperation, reconcileReadiness } from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";
import type { ReadinessOperation, ReadinessReconcilePayload } from "@/lib/types";
import { useUiStore } from "@/stores/ui-store";

const terminalStatuses = new Set([
  "completed",
  "succeeded",
  "failed",
  "error",
  "cancelled",
  "superseded",
]);

function isActiveStatus(status: string | undefined) {
  return Boolean(status) && !terminalStatuses.has(status?.toLowerCase() || "");
}

export function manualReconcilePayload(workspace: string | null): ReadinessReconcilePayload {
  return { workspace, trigger: "manual" };
}

function invalidateReadinessRoots(queryClient: ReturnType<typeof useQueryClient>) {
  queryClient.invalidateQueries({ queryKey: queryKeys.agentsSummary });
  queryClient.invalidateQueries({ queryKey: queryKeys.providers });
  queryClient.invalidateQueries({ queryKey: ["providers", "catalog"] });
  queryClient.invalidateQueries({ queryKey: ["workspaces", "providers"] });
  queryClient.invalidateQueries({ queryKey: queryKeys.sessionsRoot });
  queryClient.invalidateQueries({ queryKey: queryKeys.home });
  queryClient.invalidateQueries({ queryKey: queryKeys.skillsRoot });
  queryClient.invalidateQueries({ queryKey: queryKeys.statsRoot });
  queryClient.invalidateQueries({ queryKey: queryKeys.workspaces });
  queryClient.invalidateQueries({ queryKey: queryKeys.workspacesWithSessionsRoot });
}

export function useReadiness(options: { startOnMount?: boolean } = {}) {
  const { startOnMount = true } = options;
  const queryClient = useQueryClient();
  const selectedWorkspace = useUiStore((state) => state.selectedWorkspace);
  const meta = useQuery({ queryKey: queryKeys.meta, queryFn: getMeta });
  const workspace = selectedWorkspace || meta.data?.selected_workspace || null;
  const startupAttemptedFor = useRef<string | null>(null);
  const [startedOperation, setStartedOperation] = useState<{ workspaceKey: string; id: string } | null>(null);

  const readiness = useQuery({
    queryKey: queryKeys.readiness(workspace),
    queryFn: () => getReadiness(workspace),
    enabled: !meta.isLoading,
  });

  const reconcile = useMutation({
    mutationFn: reconcileReadiness,
    onSuccess: (result) => {
      if (result.operation_id) setStartedOperation({ workspaceKey: workspace || "__default__", id: result.operation_id });
      queryClient.setQueryData(queryKeys.readiness(workspace), result.readiness);
      if (result.disposition === "noop") invalidateReadinessRoots(queryClient);
    },
  });

  // Auto-start a reconcile when the backend reports one is needed. The backend
  // owns the full/incremental/none verdict via reconcile_required — no more
  // localStorage guessing. Startup trigger is Steady: never a full rebuild.
  useEffect(() => {
    if (!startOnMount || meta.isLoading || readiness.isLoading || !readiness.data) return;
    const required = readiness.data.reconcile_required;
    const activeId = readiness.data.active_operation_id;
    const workspaceKey = workspace || "__default__";
    if (required && required !== "none" && !activeId && startupAttemptedFor.current !== workspaceKey) {
      startupAttemptedFor.current = workspaceKey;
      reconcile.mutate({ workspace, trigger: "startup" });
    }
  }, [startOnMount, meta.isLoading, readiness.isLoading, readiness.data, workspace, reconcile]);

  useEffect(() => {
    if (startupAttemptedFor.current !== (workspace || "__default__")) {
      startupAttemptedFor.current = null;
    }
  }, [workspace]);

  const operationId = readiness.data?.active_operation_id ||
    (startedOperation?.workspaceKey === (workspace || "__default__") ? startedOperation.id : null);

  const operationKey = queryKeys.readinessOperation(operationId || "");
  const cachedOperation = queryClient.getQueryData<ReadinessOperation>(operationKey);
  const operation = useQuery({
    queryKey: operationKey,
    queryFn: () => getReadinessOperation(operationId!),
    enabled: Boolean(operationId) && !isReadinessOperationTerminal(cachedOperation),
    refetchInterval: (query) =>
      isActiveStatus(query.state.data?.status) ? 2000 : false,
  });

  useEffect(() => {
    if (!operation.data || isActiveStatus(operation.data.status)) return;
    queryClient.setQueryData(queryKeys.readiness(workspace), operation.data.readiness);
    invalidateReadinessRoots(queryClient);
  }, [operation.data, queryClient, workspace]);

  const effectiveReadiness = operation.data?.readiness || readiness.data;
  const status = operation.data?.status || (effectiveReadiness?.state === "ready" ? "ready" : "partial");

  return useMemo(() => ({
    workspace,
    readiness,
    operation,
    effectiveReadiness,
    status,
    isRunning: Boolean(readiness.data?.active_operation_id) || isActiveStatus(operation.data?.status),
    reconcile: (payload: ReadinessReconcilePayload) =>
      reconcile.mutate({
        ...payload,
        workspace: payload.workspace ?? workspace,
      }),
    isReconciling: reconcile.isPending,
  }), [workspace, readiness, operation, effectiveReadiness, status, reconcile]);
}

export function isReadinessOperationTerminal(operation: ReadinessOperation | undefined) {
  return Boolean(operation && !isActiveStatus(operation.status));
}
