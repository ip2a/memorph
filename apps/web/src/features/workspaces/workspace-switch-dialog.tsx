import { FormEvent, useMemo, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { FolderOpenIcon, ListIcon, Trash2Icon } from "lucide-react";
import { toast } from "sonner";
import { DialogForm, DialogFormFooter } from "@/components/shared/dialog-form";
import { PathText } from "@/components/shared/path-text";
import { workspaceName } from "@/components/shared/workspace-name";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyTitle,
} from "@/components/ui/empty";
import { Field, FieldDescription, FieldGroup, FieldLabel } from "@/components/ui/field";
import {
  InputGroup,
  InputGroupAddon,
  InputGroupButton,
  InputGroupInput,
} from "@/components/ui/input-group";
import {
  Popover,
  PopoverContent,
  PopoverDescription,
  PopoverHeader,
  PopoverTitle,
  PopoverTrigger,
} from "@/components/ui/popover";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Spinner } from "@/components/ui/spinner";
import { deleteWorkspaceHistory, getManagerWorkspaces, getMeta, listSessions, listWorkspaces, selectFolder } from "@/lib/api";
import { formatDateTime } from "@/lib/format";
import { queryKeys } from "@/lib/query-keys";
import type { ManagerWorkspaceItem, WorkspaceEntry } from "@/lib/types";
import { useUiStore } from "@/stores/ui-store";

function normalizeWorkspacePath(path: string) {
  return path.replace(/[\\/]+$/, "");
}

function isValidWorkspacePath(path: string) {
  const normalized = path.trim();
  return normalized.length > 0 && normalized !== "—" && normalized !== "-";
}

function workspaceSessionCounts(items: ManagerWorkspaceItem[] | undefined) {
  const counts = new Map<string, number>();
  for (const item of items ?? []) {
    const key = normalizeWorkspacePath(item.workspace);
    counts.set(key, (counts.get(key) ?? 0) + item.session_count);
  }
  return counts;
}

type AggregatedWorkspace = {
  path: string;
  sessionCount: number;
  lastActiveAt: number | null;
};

function aggregateWorkspacesWithSessions(items: ManagerWorkspaceItem[] | undefined): AggregatedWorkspace[] {
  const map = new Map<string, AggregatedWorkspace>();

  for (const item of items ?? []) {
    if (item.session_count <= 0 || !isValidWorkspacePath(item.workspace)) continue;

    const key = normalizeWorkspacePath(item.workspace);
    const lastActive = item.last_active_at ?? null;
    const existing = map.get(key);

    if (existing) {
      existing.sessionCount += item.session_count;
      if (lastActive !== null) {
        existing.lastActiveAt =
          existing.lastActiveAt === null ? lastActive : Math.max(existing.lastActiveAt, lastActive);
      }
      continue;
    }

    map.set(key, {
      path: item.workspace,
      sessionCount: item.session_count,
      lastActiveAt: lastActive,
    });
  }

  return Array.from(map.values()).sort((left, right) => (right.lastActiveAt ?? 0) - (left.lastActiveAt ?? 0));
}

function WorkspaceSessionPickerPanel({
  isLoading,
  workspaces,
  onPick,
}: {
  isLoading: boolean;
  workspaces: AggregatedWorkspace[];
  onPick: (workspace: string) => void;
}) {
  return (
    <ScrollArea className="h-72">
      <div className="flex flex-col gap-0.5 p-2">
        {isLoading ? (
          <div className="flex min-h-28 items-center justify-center gap-2 text-sm text-muted-foreground">
            <Spinner />
            Loading
          </div>
        ) : workspaces.length ? (
          workspaces.map((workspace) => (
            <WorkspaceSessionPickerRow
              key={normalizeWorkspacePath(workspace.path)}
              workspace={workspace}
              onPick={onPick}
            />
          ))
        ) : (
          <Empty className="min-h-32 border-0">
            <EmptyHeader>
              <EmptyTitle>No sessions found</EmptyTitle>
              <EmptyDescription>Install an agent and create sessions, or paste a path manually.</EmptyDescription>
            </EmptyHeader>
          </Empty>
        )}
      </div>
    </ScrollArea>
  );
}

function WorkspaceSessionPickerRow({
  workspace,
  onPick,
}: {
  workspace: AggregatedWorkspace;
  onPick: (workspace: string) => void;
}) {
  return (
    <button
      type="button"
      className="flex w-full flex-col gap-1 rounded-md px-2 py-2 text-left outline-none transition-colors hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring/50"
      data-workspace-session-picker-item
      onClick={() => onPick(workspace.path)}
    >
      <span className="flex min-w-0 items-center justify-between gap-3">
        <strong className="truncate">{workspaceName(workspace.path, "memorph")}</strong>
        <span className="flex shrink-0 items-center gap-3 font-mono text-xs text-muted-foreground">
          <span>{workspace.sessionCount} sessions</span>
          {workspace.lastActiveAt ? <span>{formatDateTime(workspace.lastActiveAt)}</span> : null}
        </span>
      </span>
      <PathText value={workspace.path} wrap="all" />
    </button>
  );
}

function WorkspaceHistoryRow({
  workspace,
  sessionCount,
  isRemoving,
  onPick,
  onRemove,
}: {
  workspace: WorkspaceEntry;
  sessionCount?: number;
  isRemoving: boolean;
  onPick: (workspace: string) => void;
  onRemove: (workspace: string) => void;
}) {
  return (
    <div
      className="grid grid-cols-[minmax(0,1fr)_auto] items-stretch border-b last:border-b-0 rounded-md transition-all hover:bg-muted has-[button:active]:translate-y-px has-[button:active]:bg-muted/80"
      data-workspace-switch-item
    >
      <button
        type="button"
        className="min-w-0 w-full cursor-pointer rounded-l-md px-2 py-2 text-left outline-none focus-visible:ring-2 focus-visible:ring-ring/50 focus-visible:ring-inset"
        onClick={() => onPick(workspace.path)}
      >
        <span className="grid min-w-0 w-full gap-1">
          <span className="grid min-w-0 grid-cols-[minmax(0,1fr)_auto] items-center gap-3">
            <strong className="truncate">{workspaceName(workspace.path, "memorph")}</strong>
            <span className="flex shrink-0 items-center gap-3 font-mono text-xs text-muted-foreground">
              {sessionCount !== undefined ? <span>{sessionCount} sessions</span> : null}
              <span>{formatDateTime(workspace.last_viewed_at)}</span>
            </span>
          </span>
          <PathText value={workspace.path} wrap="all" />
        </span>
      </button>
      <Button
        type="button"
        variant="ghost"
        className="h-auto shrink-0 self-stretch rounded-none rounded-r-md active:translate-y-0 hover:bg-destructive/10 hover:text-destructive"
        disabled={isRemoving}
        onClick={() => onRemove(workspace.path)}
      >
        {isRemoving ? <Spinner data-icon="inline-start" /> : <Trash2Icon data-icon="inline-start" />}
        Remove
      </Button>
    </div>
  );
}

export function WorkspaceSwitchDialog({ open, onOpenChange }: { open: boolean; onOpenChange: (open: boolean) => void }) {
  const queryClient = useQueryClient();
  const selectedWorkspace = useUiStore((state) => state.selectedWorkspace);
  const setSelectedWorkspace = useUiStore((state) => state.setSelectedWorkspace);
  const [draft, setDraft] = useState<string | null>(null);
  const [sessionPickerOpen, setSessionPickerOpen] = useState(false);

  const sessionWorkspaceFilter = useMemo(() => ({ sort: "recent" as const }), []);

  const meta = useQuery({
    queryKey: queryKeys.meta,
    queryFn: getMeta,
  });

  const workspaces = useQuery({
    queryKey: queryKeys.workspaces,
    queryFn: listWorkspaces,
    enabled: open,
    initialData: () => meta.data?.workspaces,
  });

  const managerWorkspaces = useQuery({
    queryKey: queryKeys.manager("workspaces", sessionWorkspaceFilter),
    queryFn: () => getManagerWorkspaces(sessionWorkspaceFilter),
    enabled: open,
  });

  const currentWorkspace = selectedWorkspace || meta.data?.selected_workspace || "";
  const draftWorkspace = draft ?? currentWorkspace;
  const workspaceItems = useMemo(() => workspaces.data ?? meta.data?.workspaces ?? [], [meta.data?.workspaces, workspaces.data]);
  const sessionCounts = useMemo(() => workspaceSessionCounts(managerWorkspaces.data?.items), [managerWorkspaces.data?.items]);
  const workspacesWithSessions = useMemo(
    () => aggregateWorkspacesWithSessions(managerWorkspaces.data?.items),
    [managerWorkspaces.data?.items],
  );

  const switchWorkspace = useMutation({
    mutationFn: async (workspace: string) => {
      await listSessions({ all: true, details: true, limit: 1, workspace });
      return workspace;
    },
    onSuccess: (workspace) => {
      setSelectedWorkspace(workspace);
      onOpenChange(false);
      toast.success("Workspace switched", { description: workspaceName(workspace, "memorph") });
      // Don't await — React Query keeps isPending true until async onSuccess settles,
      // so waiting on a full-cache refetch left the Go spinner spinning after the switch.
      void queryClient.invalidateQueries();
    },
  });

  const removeWorkspace = useMutation({
    mutationFn: deleteWorkspaceHistory,
    onSuccess: (nextWorkspaces, removedWorkspace) => {
      queryClient.setQueryData(queryKeys.workspaces, nextWorkspaces);
      queryClient.setQueryData(
        queryKeys.meta,
        meta.data
          ? {
              ...meta.data,
              selected_workspace: currentWorkspace === removedWorkspace ? null : meta.data.selected_workspace,
              workspaces: nextWorkspaces,
            }
          : meta.data,
      );
      if (currentWorkspace === removedWorkspace) setSelectedWorkspace(null);
      toast.success("Workspace history removed", { description: workspaceName(removedWorkspace, "memorph") });
    },
  });

  function submitWorkspace(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const workspace = draftWorkspace.trim();
    if (!workspace) return;
    switchWorkspace.mutate(workspace);
  }

  function pickWorkspace(workspace: string) {
    setDraft(workspace);
    switchWorkspace.mutate(workspace);
  }

  function pickExistingWorkspace(workspace: string) {
    setSessionPickerOpen(false);
    pickWorkspace(workspace);
  }

  async function browseFolder() {
    try {
      const result = await selectFolder({
        start_path: draftWorkspace.trim() || currentWorkspace || null,
      });
      if (result.path) setDraft(result.path);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      toast.error("Error", {
        description: /only available in the desktop app/i.test(message)
          ? "Folder picker is only available in the desktop app."
          : message,
      });
    }
  }

  return (
    <Dialog
      open={open}
      onOpenChange={(nextOpen) => {
        if (!nextOpen) {
          setDraft(null);
          setSessionPickerOpen(false);
        }
        onOpenChange(nextOpen);
      }}
    >
      <DialogContent className="sm:max-w-2xl" data-workspace-switch-dialog>
        <DialogHeader>
          <DialogTitle>Switch Workspace</DialogTitle>
          <DialogDescription>Choose a known workspace or enter a path to load its sessions.</DialogDescription>
        </DialogHeader>

        <DialogForm onSubmit={submitWorkspace}>
          <FieldGroup>
            <Field>
              <FieldLabel htmlFor="workspace-switch-input">Workspace Path</FieldLabel>
              <InputGroup>
                <InputGroupInput
                  id="workspace-switch-input"
                  name="workspace"
                  list="known-workspaces"
                  value={draftWorkspace}
                  placeholder={workspaceItems[0]?.path || ""}
                  onChange={(event) => setDraft(event.target.value)}
                />
                <InputGroupAddon align="inline-end">
                  <InputGroupButton type="button" variant="ghost" onClick={() => void browseFolder()}>
                    <FolderOpenIcon data-icon="inline-start" />
                    Browse
                  </InputGroupButton>
                </InputGroupAddon>
              </InputGroup>
              <FieldDescription>Use an existing workspace path or paste another local project path.</FieldDescription>
            </Field>
          </FieldGroup>

          <datalist id="known-workspaces">
            {workspaceItems.map((workspace) => (
              <option key={workspace.path} value={workspace.path} />
            ))}
          </datalist>

          <section className="flex min-h-0 flex-col gap-2" data-workspace-switch-list>
            <div className="flex items-center justify-between gap-3">
              <strong className="text-sm">Workspace History</strong>
              <div className="flex items-center gap-2">
                <Popover open={sessionPickerOpen} onOpenChange={setSessionPickerOpen}>
                  <PopoverTrigger asChild>
                    <Button type="button" variant="outline" size="sm">
                      <ListIcon data-icon="inline-start" />
                      Pick workspace
                    </Button>
                  </PopoverTrigger>
                  <PopoverContent className="w-[min(32rem,calc(100vw-2rem))] gap-0 p-0" align="end" sideOffset={6}>
                    <PopoverHeader className="border-b px-3 py-2.5">
                      <PopoverTitle>Workspaces with sessions</PopoverTitle>
                      <PopoverDescription>Pick a workspace path detected from installed agents.</PopoverDescription>
                    </PopoverHeader>
                    <WorkspaceSessionPickerPanel
                      isLoading={managerWorkspaces.isLoading}
                      workspaces={workspacesWithSessions}
                      onPick={pickExistingWorkspace}
                    />
                  </PopoverContent>
                </Popover>
                <span className="font-mono text-xs text-muted-foreground">{workspaceItems.length}</span>
              </div>
            </div>
            <ScrollArea className="h-72 rounded-md border">
              <div className="px-3">
                {workspaces.isLoading ? (
                  <div className="flex min-h-28 items-center justify-center gap-2 text-sm text-muted-foreground">
                    <Spinner />
                    Loading
                  </div>
                ) : workspaceItems.length ? (
                  workspaceItems.map((workspace) => (
                    <WorkspaceHistoryRow
                      key={workspace.path}
                      workspace={workspace}
                      sessionCount={
                        managerWorkspaces.data
                          ? sessionCounts.get(normalizeWorkspacePath(workspace.path)) ?? 0
                          : undefined
                      }
                      isRemoving={removeWorkspace.isPending && removeWorkspace.variables === workspace.path}
                      onPick={pickWorkspace}
                      onRemove={(path) => removeWorkspace.mutate(path)}
                    />
                  ))
                ) : (
                  <Empty className="min-h-36">
                    <EmptyHeader>
                      <EmptyTitle>No Workspace</EmptyTitle>
                      <EmptyDescription>Recent workspaces will appear here after you switch.</EmptyDescription>
                    </EmptyHeader>
                  </Empty>
                )}
              </div>
            </ScrollArea>
          </section>

          <DialogFormFooter
            onCancel={() => onOpenChange(false)}
            submitDisabled={!draftWorkspace.trim()}
            submitLabel="Go"
            submitting={switchWorkspace.isPending}
          />
        </DialogForm>
      </DialogContent>
    </Dialog>
  );
}
