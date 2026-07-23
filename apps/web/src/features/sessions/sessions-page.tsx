import { useMemo, useState } from "react";
import { Link } from "react-router-dom";
import { ArrowRightIcon, EyeIcon, PinIcon, RefreshCwIcon, RotateCwIcon, SearchIcon, TriangleAlertIcon } from "lucide-react";
import { toast } from "sonner";
import { PageEmpty, PageError, PageSkeleton } from "@/components/shared/page-states";
import { PathText } from "@/components/shared/path-text";
import { ProviderLogo } from "@/components/shared/provider-logo";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { formatBytes, formatDateTime, sessionTitle } from "@/lib/format";
import type { SessionHookFilter, SessionItem, SessionListSort } from "@/lib/types";
import { useRefreshSessionStaleness, useReprojectStaleSessions, useSessions } from "@/features/sessions/queries";

function matchesSearch(session: SessionItem, query: string) {
  if (!query) return true;
  const text = [session.session_id, session.title, session.native_title, session.display_title, session.project_dir]
    .filter(Boolean)
    .join(" ")
    .toLowerCase();
  return text.includes(query.toLowerCase());
}

export function SessionsPage() {
  const [search, setSearch] = useState("");
  const [sort, setSort] = useState<SessionListSort>("recent");
  const [hookFilter, setHookFilter] = useState<SessionHookFilter>("all");
  const [page, setPage] = useState(0);
  const pageSize = 25;

  const params = useMemo(
    () => ({ all: true, details: true, limit: pageSize, offset: page * pageSize, sort, hook_filter: hookFilter }),
    [hookFilter, page, sort],
  );
  const sessions = useSessions(params);
  const refreshStaleness = useRefreshSessionStaleness();
  const reprojectStale = useReprojectStaleSessions();

  if (sessions.isLoading) return <PageSkeleton />;
  if (sessions.error) return <PageError title="Sessions failed to load" message={sessions.error.message} />;

  const groups = (sessions.data?.groups ?? [])
    .map((group) => ({
      ...group,
      sessions: group.sessions.filter((session) => matchesSearch(session, search)),
    }))
    .filter((group) => group.sessions.length > 0);
  const total = groups.reduce((sum, group) => sum + group.sessions.length, 0);
  const staleTotal = groups.reduce(
    (sum, group) => sum + group.sessions.filter((session) => session.stale).length,
    0,
  );

  function handleRefreshStaleness() {
    refreshStaleness.mutate(undefined, {
      onSuccess: (report) => {
        toast.success(
          `Checked ${report.checked_sources} sources: ${report.stale_snapshots} stale, ${report.fresh_snapshots} fresh`,
        );
      },
      onError: (error) => toast.error(error.message),
    });
  }

  function handleReprojectStale() {
    reprojectStale.mutate(null, {
      onSuccess: (report) => {
        const summary = `${report.reprojected_snapshots}/${report.candidate_snapshots} snapshots reprojected`;
        if (report.failed_snapshots || report.missing_sources || report.unsupported_providers) {
          toast.warning(summary);
        } else {
          toast.success(summary);
        }
      },
      onError: (error) => toast.error(error.message),
    });
  }

  return (
    <div className="flex flex-col gap-6">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="flex flex-col gap-2">
          <Badge variant="secondary">Session List</Badge>
          <h1 className="text-3xl font-semibold">Sessions</h1>
          <p className="text-muted-foreground">Provider-scoped sessions rebuilt as a shadcn table workflow.</p>
        </div>
        <div className="flex flex-wrap gap-2">
          <Button type="button" variant="outline" disabled={refreshStaleness.isPending} onClick={handleRefreshStaleness}>
            <RefreshCwIcon className={refreshStaleness.isPending ? "animate-spin" : undefined} data-icon="inline-start" />
            Check sources
          </Button>
          <Button type="button" variant="outline" disabled={reprojectStale.isPending || staleTotal === 0} onClick={handleReprojectStale}>
            <RotateCwIcon className={reprojectStale.isPending ? "animate-spin" : undefined} data-icon="inline-start" />
            Reproject stale
          </Button>
          <Button asChild variant="outline">
            <Link to="/manager">
              Manager
              <ArrowRightIcon data-icon="inline-end" />
            </Link>
          </Button>
        </div>
      </div>

      <Card>
        <CardHeader>
          <CardTitle>Filters</CardTitle>
          <CardDescription>
            {total} sessions across {groups.length} providers
            {staleTotal ? ` · ${staleTotal} stale` : ""}
          </CardDescription>
        </CardHeader>
        <CardContent>
          <div className="grid gap-3 md:grid-cols-[1fr_auto_auto]">
            <div className="relative">
              <SearchIcon className="pointer-events-none absolute left-2.5 top-2.5 text-muted-foreground" />
              <Input
                className="pl-8"
                value={search}
                onChange={(event) => setSearch(event.target.value)}
                placeholder="Search title, id, or workspace"
              />
            </div>
            <Select value={sort} onValueChange={(value) => {
              setSort(value as SessionListSort);
              setPage(0);
            }}>
              <SelectTrigger>
                <SelectValue placeholder="Sort" />
              </SelectTrigger>
              <SelectContent>
                <SelectGroup>
                  <SelectItem value="recent">Recent</SelectItem>
                  <SelectItem value="title">Title</SelectItem>
                  <SelectItem value="hook_attention">Hook attention</SelectItem>
                </SelectGroup>
              </SelectContent>
            </Select>
            <Select value={hookFilter} onValueChange={(value) => {
              setHookFilter(value as SessionHookFilter);
              setPage(0);
            }}>
              <SelectTrigger>
                <SelectValue placeholder="Hook filter" />
              </SelectTrigger>
              <SelectContent>
                <SelectGroup>
                  <SelectItem value="all">All hooks</SelectItem>
                  <SelectItem value="attention">Attention</SelectItem>
                  <SelectItem value="runtime">Runtime</SelectItem>
                  <SelectItem value="linked">Linked</SelectItem>
                  <SelectItem value="no_hook">No hook</SelectItem>
                </SelectGroup>
              </SelectContent>
            </Select>
          </div>
        </CardContent>
      </Card>

      {total === 0 ? (
        <PageEmpty title="No sessions matched" description="Change filters or switch workspace to inspect provider sessions." />
      ) : (
        <>
          {groups.map((group) => (
          <Card key={group.provider_id}>
            <CardHeader>
              <CardTitle className="flex items-center gap-2">
                <ProviderLogo providerId={group.provider_id} size="sm" alt={group.provider_name || group.provider_id} />
                <span className="truncate">{group.provider_name || group.provider_id}</span>
              </CardTitle>
              <CardDescription>{group.sessions.length} sessions</CardDescription>
            </CardHeader>
            <CardContent>
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>Title</TableHead>
                    <TableHead>Status</TableHead>
                    <TableHead>Workspace</TableHead>
                    <TableHead>Updated</TableHead>
                    <TableHead className="text-right">Messages</TableHead>
                    <TableHead className="text-right">Size</TableHead>
                    <TableHead className="text-right">Open</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {group.sessions.map((session) => (
                    <TableRow key={session.session_id}>
                      <TableCell>
                        <div className="flex min-w-0 flex-col gap-1">
                          <span className="truncate font-medium">{sessionTitle(session)}</span>
                          <span className="truncate text-muted-foreground">{session.session_id}</span>
                        </div>
                      </TableCell>
                      <TableCell>
                        <div className="flex flex-wrap gap-1">
                          {session.pinned ? <Badge variant="secondary"><PinIcon />Pinned</Badge> : null}
                          {session.hidden ? <Badge variant="outline"><EyeIcon />Hidden</Badge> : null}
                          {session.stale ? <Badge variant="destructive"><TriangleAlertIcon />Stale</Badge> : null}
                          {session.hook_runtime_summary ? <Badge variant="outline">Hook</Badge> : null}
                        </div>
                      </TableCell>
                      <TableCell><PathText value={session.project_dir} wrap="all" /></TableCell>
                      <TableCell>{formatDateTime(session.last_active_at)}</TableCell>
                      <TableCell className="text-right">{session.message_count ?? "-"}</TableCell>
                      <TableCell className="text-right">{formatBytes(session.size_bytes)}</TableCell>
                      <TableCell className="text-right">
                        <Button asChild variant="ghost">
                          <Link to={`/sessions/${encodeURIComponent(session.provider_id)}/${encodeURIComponent(session.session_id)}`}>
                            Detail
                            <ArrowRightIcon data-icon="inline-end" />
                          </Link>
                        </Button>
                      </TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            </CardContent>
          </Card>
          ))}
          <div className="flex items-center justify-end gap-2">
            <span className="text-muted-foreground text-sm">Page {page + 1}</span>
            <Button type="button" variant="outline" disabled={page === 0 || sessions.isFetching} onClick={() => setPage((current) => current - 1)}>
              Previous
            </Button>
            <Button type="button" variant="outline" disabled={!sessions.data?.has_more || sessions.isFetching} onClick={() => setPage((current) => current + 1)}>
              Next
            </Button>
          </div>
        </>
      )}
    </div>
  );
}
