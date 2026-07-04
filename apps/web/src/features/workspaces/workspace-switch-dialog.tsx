import { FormEvent, useMemo, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { FolderOpenIcon, Trash2Icon } from "lucide-react";
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
import { ScrollArea } from "@/components/ui/scroll-area";
import { Spinner } from "@/components/ui/spinner";
import { deleteWorkspaceHistory, getMeta, listSessions, listWorkspaces } from "@/lib/api";
import { formatDateTime } from "@/lib/format";
import { queryKeys } from "@/lib/query-keys";
import type { WorkspaceEntry } from "@/lib/types";
import { useUiStore } from "@/stores/ui-store";

function WorkspaceHistoryRow({
  workspace,
  isRemoving,
  onPick,
  onRemove,
}: {
  workspace: WorkspaceEntry;
  isRemoving: boolean;
  onPick: (workspace: string) => void;
  onRemove: (workspace: string) => void;
}) {
  return (
    <div className="grid grid-cols-[minmax(0,1fr)_auto] items-stretch gap-2 border-b py-2 last:border-b-0" data-workspace-switch-item>
      <Button
        type="button"
        variant="ghost"
        className="h-auto min-w-0 justify-start px-2 py-2 text-left"
        onClick={() => onPick(workspace.path)}
      >
        <span className="grid min-w-0 gap-1">
          <span className="flex min-w-0 items-baseline justify-between gap-3">
            <strong className="truncate">{workspaceName(workspace.path, "memorph")}</strong>
            <span className="shrink-0 font-mono text-xs text-muted-foreground">{formatDateTime(workspace.last_viewed_at)}</span>
          </span>
          <PathText value={workspace.path} wrap="all" />
        </span>
      </Button>
      <Button
        type="button"
        variant="ghost"
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

  const currentWorkspace = selectedWorkspace || meta.data?.selected_workspace || "";
  const draftWorkspace = draft ?? currentWorkspace;
  const workspaceItems = useMemo(() => workspaces.data ?? meta.data?.workspaces ?? [], [meta.data?.workspaces, workspaces.data]);

  const switchWorkspace = useMutation({
    mutationFn: async (workspace: string) => {
      await listSessions({ all: true, details: true, limit: 1, workspace });
      return workspace;
    },
    onSuccess: async (workspace) => {
      setSelectedWorkspace(workspace);
      await queryClient.invalidateQueries();
      onOpenChange(false);
      toast.success("Workspace switched", { description: workspaceName(workspace, "memorph") });
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

  return (
    <Dialog
      open={open}
      onOpenChange={(nextOpen) => {
        if (!nextOpen) setDraft(null);
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
                  <InputGroupButton type="button" variant="ghost" disabled>
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
              <span className="font-mono text-xs text-muted-foreground">{workspaceItems.length}</span>
            </div>
            <ScrollArea className="max-h-72 rounded-md border">
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
