import * as React from "react"
import { InfoIcon, SearchIcon } from "lucide-react"

import { CollapsibleToolbar, type CollapsibleToolbarEntry } from "@/components/shared/collapsible-toolbar"
import { Button } from "@/components/ui/button"
import { DropdownMenuItem } from "@/components/ui/dropdown-menu"
import { Input } from "@/components/ui/input"
import { useI18n } from "@/lib/i18n-context"

type SessionDetailHeaderActionsProps = {
  eventSearch: string
  onEventSearchChange: (value: string) => void
  onOpenArtifacts: () => void
  onOpenCompression: () => void
  onOpenDelete: () => void
  onOpenDetails: () => void
  onOpenExport: () => void
  onOpenRename: () => void
  onOpenSync: () => void
  onOpenSwitch: () => void
}

export function SessionDetailHeaderActions({
  eventSearch,
  onEventSearchChange,
  onOpenArtifacts,
  onOpenCompression,
  onOpenDelete,
  onOpenDetails,
  onOpenExport,
  onOpenRename,
  onOpenSync,
  onOpenSwitch,
}: SessionDetailHeaderActionsProps) {
  const { t } = useI18n()

  const entries = React.useMemo<CollapsibleToolbarEntry[]>(
    () => [
      {
        id: "details",
        collapsePriority: 10,
        renderButton: () => (
          <Button type="button" variant="outline" size="sm" onClick={onOpenDetails}>
            <InfoIcon data-icon="inline-start" />
            Details
          </Button>
        ),
        renderMenuItem: () => (
          <DropdownMenuItem onSelect={onOpenDetails}>
            <InfoIcon />
            Details
          </DropdownMenuItem>
        ),
      },
      {
        id: "artifacts",
        collapsePriority: 11,
        renderButton: () => (
          <Button type="button" variant="outline" size="sm" onClick={onOpenArtifacts}>
            Artifacts
          </Button>
        ),
        renderMenuItem: () => <DropdownMenuItem onSelect={onOpenArtifacts}>Artifacts</DropdownMenuItem>,
      },
      {
        id: "compression",
        collapsePriority: 12,
        renderButton: () => (
          <Button type="button" variant="outline" size="sm" onClick={onOpenCompression}>
            Compression
          </Button>
        ),
        renderMenuItem: () => <DropdownMenuItem onSelect={onOpenCompression}>Compression</DropdownMenuItem>,
      },
      {
        id: "sync",
        collapsePriority: 13,
        renderButton: () => (
          <Button type="button" variant="outline" size="sm" onClick={onOpenSync}>
            Sync
          </Button>
        ),
        renderMenuItem: () => <DropdownMenuItem onSelect={onOpenSync}>Sync</DropdownMenuItem>,
      },
      {
        id: "switch",
        collapsePriority: 14,
        renderButton: () => (
          <Button type="button" variant="outline" size="sm" onClick={onOpenSwitch}>
            Switch
          </Button>
        ),
        renderMenuItem: () => <DropdownMenuItem onSelect={onOpenSwitch}>Switch</DropdownMenuItem>,
      },
      {
        id: "export",
        collapsePriority: 15,
        renderButton: () => (
          <Button type="button" variant="outline" size="sm" onClick={onOpenExport}>
            Export
          </Button>
        ),
        renderMenuItem: () => <DropdownMenuItem onSelect={onOpenExport}>Export</DropdownMenuItem>,
      },
      {
        id: "rename",
        collapsePriority: 16,
        renderButton: () => (
          <Button type="button" variant="outline" size="sm" onClick={onOpenRename}>
            Rename
          </Button>
        ),
        renderMenuItem: () => <DropdownMenuItem onSelect={onOpenRename}>Rename</DropdownMenuItem>,
      },
      {
        id: "remove",
        collapsePriority: 55,
        renderButton: () => (
          <Button type="button" variant="destructive" size="sm" onClick={onOpenDelete}>
            Remove
          </Button>
        ),
        renderMenuItem: () => (
          <DropdownMenuItem variant="destructive" onSelect={onOpenDelete}>
            Remove
          </DropdownMenuItem>
        ),
      },
    ],
    [
      onOpenArtifacts,
      onOpenCompression,
      onOpenDelete,
      onOpenDetails,
      onOpenExport,
      onOpenRename,
      onOpenSync,
      onOpenSwitch,
    ],
  )

  return (
    <div className="flex w-full min-w-0 items-center gap-3">
      <div className="relative w-full max-w-xs min-w-[10rem] shrink-0">
        <SearchIcon className="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground" />
        <Input
          className="h-8 pl-8"
          value={eventSearch}
          onChange={(event) => onEventSearchChange(event.target.value)}
          placeholder="Search events"
          data-session-event-search
        />
      </div>
      <CollapsibleToolbar className="min-w-0 flex-1" entries={entries} moreLabel={t("more")} />
    </div>
  )
}
