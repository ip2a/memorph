import { FormEvent, useCallback, useEffect, useMemo, useState } from "react";
import { keepPreviousData, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ChevronLeftIcon, ChevronRightIcon, FolderOpenIcon, SearchIcon, Trash2Icon } from "lucide-react";
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
import { Empty, EmptyDescription, EmptyHeader, EmptyTitle } from "@/components/ui/empty";
import {
  InputGroup,
  InputGroupAddon,
  InputGroupInput,
} from "@/components/ui/input-group";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Spinner } from "@/components/ui/spinner";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { PathBrowser } from "@/features/workspaces/path-browser";
import {
  deleteWorkspaceHistory,
  getManagerWorkspaces,
  getMeta,
  listSessions,
  listWorkspaces,
  listWorkspacesWithSessions,
  selectFolder,
} from "@/lib/api";
import { formatDateTime } from "@/lib/format";
import { queryKeys } from "@/lib/query-keys";
import type { ManagerWorkspaceItem, WorkspaceEntry, WorkspaceWithSessionsItem } from "@/lib/types";
import { useUiStore } from "@/stores/ui-store";

const PICK_WORKSPACE_PAGE_SIZE = 5;

function normalizeWorkspacePath(path: string) {
  return path.replace(/[\\/]+$/, "");
}

function workspaceSessionCounts(items: ManagerWorkspaceItem[] | undefined) {
  const counts = new Map<string, number>();
  for (const item of items ?? []) {
    const key = normalizeWorkspacePath(item.workspace);
    counts.set(key, (counts.get(key) ?? 0) + item.session_count);
  }
  return counts;
}

function matchesWorkspaceSearch(path: string, search: string) {
  const needle = search.trim().toLocaleLowerCase();
  if (!needle) return true;
  const name = workspaceName(path, "memorph").toLocaleLowerCase();
  return name.includes(needle) || path.toLocaleLowerCase().includes(needle);
}

function WorkspaceSessionPickerPanel({
  enabled,
  search,
  onPick,
}: {
  enabled: boolean;
  search: string;
  onPick: (workspace: string) => void;
}) {
  const [debouncedSearch, setDebouncedSearch] = useState(search);
  const [page, setPage] = useState(1);

  useEffect(() => {
    const timer = window.setTimeout(() => setDebouncedSearch(search), 300);
    return () => window.clearTimeout(timer);
  }, [search]);

  useEffect(() => {
    setPage(1);
  }, [debouncedSearch]);

  const picker = useQuery({
    queryKey: queryKeys.workspacesWithSessions({
      q: debouncedSearch || undefined,
      page,
      page_size: PICK_WORKSPACE_PAGE_SIZE,
    }),
    queryFn: () =>
      listWorkspacesWithSessions({
        q: debouncedSearch || undefined,
        page,
        page_size: PICK_WORKSPACE_PAGE_SIZE,
      }),
    enabled,
    placeholderData: keepPreviousData,
  });

  const pageItems = picker.data?.items ?? [];
  const currentPage = picker.data?.page ?? page;
  const totalPages = picker.data?.total_pages ?? 1;
  const totalCount = picker.data?.total_count ?? 0;
  const isLoading = picker.isLoading && !picker.data;
  const canPrev = currentPage > 1 && !picker.isFetching;
  const canNext = currentPage < totalPages && !picker.isFetching;
  const showPager = totalCount > PICK_WORKSPACE_PAGE_SIZE;

  return (
    <section className="flex min-h-0 flex-1 flex-col gap-2" data-workspace-session-picker>
      <div className="flex shrink-0 items-center justify-between gap-3">
        <strong className="text-sm">With Sessions</strong>
        {showPager ? (
          <div className="flex items-center gap-1">
            <span className="font-mono text-xs text-muted-foreground">
              {currentPage}/{totalPages} · {totalCount}
            </span>
            <Button
              type="button"
              variant="ghost"
              size="icon-sm"
              aria-label="Previous page"
              disabled={!canPrev}
              onClick={() => setPage(currentPage - 1)}
            >
              <ChevronLeftIcon />
            </Button>
            <Button
              type="button"
              variant="ghost"
              size="icon-sm"
              aria-label="Next page"
              disabled={!canNext}
              onClick={() => setPage(currentPage + 1)}
            >
              <ChevronRightIcon />
            </Button>
          </div>
        ) : (
          <span className="font-mono text-xs text-muted-foreground">{totalCount}</span>
        )}
      </div>
      <ScrollArea className="h-full min-h-0 flex-1 rounded-md border">
        <div className="flex flex-col gap-0.5 p-2">
          {isLoading ? (
            <div className="flex min-h-28 items-center justify-center gap-2 text-sm text-muted-foreground">
              <Spinner />
              Loading
            </div>
          ) : pageItems.length ? (
            pageItems.map((workspace) => (
              <WorkspaceSessionPickerRow
                key={normalizeWorkspacePath(workspace.path)}
                workspace={workspace}
                onPick={onPick}
              />
            ))
          ) : (
            <Empty className="min-h-32 border-0">
              <EmptyHeader>
                <EmptyTitle>{debouncedSearch.trim() ? "No matches" : "No sessions found"}</EmptyTitle>
                <EmptyDescription>
                  {debouncedSearch.trim()
                    ? "Try a different name or path."
                    : "Install an agent and create sessions, or paste a path manually."}
                </EmptyDescription>
              </EmptyHeader>
            </Empty>
          )}
        </div>
      </ScrollArea>
    </section>
  );
}

function WorkspaceSessionPickerRow({
  workspace,
  onPick,
}: {
  workspace: WorkspaceWithSessionsItem;
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
          <span>{workspace.session_count} sessions</span>
          {workspace.last_active_at ? <span>{formatDateTime(workspace.last_active_at)}</span> : null}
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
  const [activeTab, setActiveTab] = useState("browse");
  const [search, setSearch] = useState("");

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
  const filteredWorkspaceItems = useMemo(
    () => workspaceItems.filter((workspace) => matchesWorkspaceSearch(workspace.path, search)),
    [search, workspaceItems],
  );
  const sessionCounts = useMemo(() => workspaceSessionCounts(managerWorkspaces.data?.items), [managerWorkspaces.data?.items]);

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

  const handlePathChange = useCallback((path: string) => setDraft(path), []);
  const handleFilterChange = useCallback((value: string) => setSearch(value), []);

  const searchPlaceholder =
    activeTab === "browse" ? "Filter directories" : activeTab === "recent" ? "Search recent" : "Search workspaces";

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
          setActiveTab("browse");
          setSearch("");
        }
        onOpenChange(nextOpen);
      }}
    >
      <DialogContent
        className="flex! h-[min(36rem,calc(100dvh-2rem))] w-full flex-col gap-4 overflow-hidden sm:max-w-2xl"
        data-workspace-switch-dialog
      >
        <DialogHeader className="shrink-0">
          <DialogTitle>Switch Workspace</DialogTitle>
          <DialogDescription>Choose a known workspace or enter a path to load its sessions.</DialogDescription>
        </DialogHeader>

        <DialogForm className="flex min-h-0 min-w-0 flex-1 flex-col" onSubmit={submitWorkspace}>
          <Tabs
            className="flex min-h-0 min-w-0 flex-1 flex-col"
            value={activeTab}
            onValueChange={setActiveTab}
          >
            <div className="flex min-w-0 shrink-0 items-center gap-2">
              <TabsList className="shrink-0">
                <TabsTrigger value="browse">Browse</TabsTrigger>
                <TabsTrigger value="recent">Recent</TabsTrigger>
                <TabsTrigger value="pick">Pick</TabsTrigger>
              </TabsList>
              <InputGroup className="min-w-0 flex-1">
                <InputGroupInput
                  aria-label="Search"
                  value={search}
                  placeholder={searchPlaceholder}
                  onChange={(event) => setSearch(event.target.value)}
                />
                <InputGroupAddon>
                  <SearchIcon />
                </InputGroupAddon>
              </InputGroup>
            </div>

            <TabsContent className="mt-0 flex min-h-0 min-w-0 flex-1 flex-col data-[state=inactive]:hidden" value="browse" forceMount>
              <PathBrowser
                active={open && activeTab === "browse"}
                filter={search}
                onFilterChange={handleFilterChange}
                initialPath={draftWorkspace || workspaceItems[0]?.path}
                onPathChange={handlePathChange}
                pathActions={
                  <Tooltip>
                    <TooltipTrigger asChild>
                      <span>
                        <Button
                          type="button"
                          variant="outline"
                          disabled={!meta.data?.capabilities.system_folder_picker}
                          onClick={() => void browseFolder()}
                        >
                          <FolderOpenIcon data-icon="inline-start" />
                          System Browse
                        </Button>
                      </span>
                    </TooltipTrigger>
                    {!meta.data?.capabilities.system_folder_picker ? (
                      <TooltipContent>System folder browsing is only available in the desktop app.</TooltipContent>
                    ) : null}
                  </Tooltip>
                }
              />
            </TabsContent>

            <TabsContent className="mt-0 flex min-h-0 min-w-0 flex-1 flex-col data-[state=inactive]:hidden" value="recent" forceMount>
              <section className="flex min-h-0 flex-1 flex-col gap-2" data-workspace-switch-list>
                <div className="flex shrink-0 items-center justify-between gap-3">
                  <strong className="text-sm">Workspace History</strong>
                  <span className="font-mono text-xs text-muted-foreground">
                    {filteredWorkspaceItems.length}
                    {search.trim() && filteredWorkspaceItems.length !== workspaceItems.length
                      ? ` / ${workspaceItems.length}`
                      : ""}
                  </span>
                </div>
                <ScrollArea className="h-full min-h-0 flex-1 rounded-md border">
                  <div className="px-3">
                    {workspaces.isLoading ? (
                      <div className="flex min-h-28 items-center justify-center gap-2 text-sm text-muted-foreground">
                        <Spinner />
                        Loading
                      </div>
                    ) : filteredWorkspaceItems.length ? (
                      filteredWorkspaceItems.map((workspace) => (
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
                          <EmptyTitle>{search.trim() ? "No matches" : "No Workspace"}</EmptyTitle>
                          <EmptyDescription>
                            {search.trim()
                              ? "Try a different name or path."
                              : "Recent workspaces will appear here after you switch."}
                          </EmptyDescription>
                        </EmptyHeader>
                      </Empty>
                    )}
                  </div>
                </ScrollArea>
              </section>
            </TabsContent>

            <TabsContent className="mt-0 flex min-h-0 min-w-0 flex-1 flex-col data-[state=inactive]:hidden" value="pick" forceMount>
              <WorkspaceSessionPickerPanel
                enabled={open && activeTab === "pick"}
                search={search}
                onPick={pickWorkspace}
              />
            </TabsContent>
          </Tabs>

          <DialogFormFooter
            className="shrink-0"
            onCancel={() => onOpenChange(false)}
            submitDisabled={!draftWorkspace.trim()}
            submitLabel="Switch to this directory"
            submitting={switchWorkspace.isPending}
          />
        </DialogForm>
      </DialogContent>
    </Dialog>
  );
}
