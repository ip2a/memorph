import { useMemo, useState } from "react";
import { Link } from "react-router-dom";
import { ArrowRightIcon, EyeIcon, PinIcon, SearchIcon } from "lucide-react";
import { PageEmpty, PageError, PageSkeleton } from "@/components/shared/page-states";
import { PathText } from "@/components/shared/path-text";
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
import { useSessions } from "@/features/sessions/queries";

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

  const params = useMemo(
    () => ({ all: true, details: true, limit: 100, sort, hook_filter: hookFilter }),
    [hookFilter, sort],
  );
  const sessions = useSessions(params);

  if (sessions.isLoading) return <PageSkeleton />;
  if (sessions.error) return <PageError title="Sessions failed to load" message={sessions.error.message} />;

  const groups = (sessions.data ?? [])
    .map((group) => ({
      ...group,
      sessions: group.sessions.filter((session) => matchesSearch(session, search)),
    }))
    .filter((group) => group.sessions.length > 0);
  const total = groups.reduce((sum, group) => sum + group.sessions.length, 0);

  return (
    <div className="flex flex-col gap-6">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="flex flex-col gap-2">
          <Badge variant="secondary">Session List</Badge>
          <h1 className="text-3xl font-semibold">Sessions</h1>
          <p className="text-muted-foreground">Provider-scoped sessions rebuilt as a shadcn table workflow.</p>
        </div>
        <Button asChild variant="outline">
          <Link to="/manager">
            Manager
            <ArrowRightIcon data-icon="inline-end" />
          </Link>
        </Button>
      </div>

      <Card>
        <CardHeader>
          <CardTitle>Filters</CardTitle>
          <CardDescription>{total} sessions across {groups.length} providers</CardDescription>
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
            <Select value={sort} onValueChange={(value) => setSort(value as SessionListSort)}>
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
            <Select value={hookFilter} onValueChange={(value) => setHookFilter(value as SessionHookFilter)}>
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
        groups.map((group) => (
          <Card key={group.provider_id}>
            <CardHeader>
              <CardTitle>{group.provider_name}</CardTitle>
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
        ))
      )}
    </div>
  );
}
