import * as React from "react"
import { InfoIcon, ListFilterIcon, SearchIcon } from "lucide-react"

import { CollapsibleToolbar, type CollapsibleToolbarEntry } from "@/components/shared/collapsible-toolbar"
import { Button } from "@/components/ui/button"
import { DropdownMenuItem } from "@/components/ui/dropdown-menu"
import { Input } from "@/components/ui/input"
import { Spinner } from "@/components/ui/spinner"
import { useI18n } from "@/lib/i18n-context"

type SessionDetailHeaderActionsProps = {
  eventSearchDraft: string
  onEventSearchDraftChange: (value: string) => void
  onEventSearchSubmit: () => void
  eventSearchPending?: boolean
  onOpenCompression: () => void
  onOpenDelete: () => void
  onOpenDetails: () => void
  onOpenExport: () => void
  onOpenFilter: () => void
  onOpenRename: () => void
  onOpenSync: () => void
  onOpenSwitch: () => void
}

export function SessionDetailHeaderActions({
  eventSearchDraft,
  onEventSearchDraftChange,
  onEventSearchSubmit,
  eventSearchPending = false,
  onOpenCompression,
  onOpenDelete,
  onOpenDetails,
  onOpenExport,
  onOpenFilter,
  onOpenRename,
  onOpenSync,
  onOpenSwitch,
}: SessionDetailHeaderActionsProps) {
  const { t } = useI18n()

  const entries = React.useMemo<CollapsibleToolbarEntry[]>(
    () => [
      {
        id: "filter",
        collapsePriority: 8,
        renderButton: () => (
          <Button type="button" variant="outline" onClick={onOpenFilter}>
            <ListFilterIcon data-icon="inline-start" />
            Filter
          </Button>
        ),
        renderMenuItem: () => (
          <DropdownMenuItem onSelect={onOpenFilter}>
            <ListFilterIcon />
            Filter
          </DropdownMenuItem>
        ),
      },
      {
        id: "details",
        collapsePriority: 10,
        renderButton: () => (
          <Button type="button" variant="outline" onClick={onOpenDetails}>
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
        id: "compression",
        collapsePriority: 12,
        renderButton: () => (
          <Button type="button" variant="outline" onClick={onOpenCompression}>
            Compression
          </Button>
        ),
        renderMenuItem: () => <DropdownMenuItem onSelect={onOpenCompression}>Compression</DropdownMenuItem>,
      },
      {
        id: "sync",
        collapsePriority: 13,
        renderButton: () => (
          <Button type="button" variant="outline" onClick={onOpenSync}>
            Sync
          </Button>
        ),
        renderMenuItem: () => <DropdownMenuItem onSelect={onOpenSync}>Sync</DropdownMenuItem>,
      },
      {
        id: "switch",
        collapsePriority: 14,
        renderButton: () => (
          <Button type="button" variant="outline" onClick={onOpenSwitch}>
            {t("switch")}
          </Button>
        ),
        renderMenuItem: () => <DropdownMenuItem onSelect={onOpenSwitch}>{t("switch")}</DropdownMenuItem>,
      },
      {
        id: "export",
        collapsePriority: 15,
        renderButton: () => (
          <Button type="button" variant="outline" onClick={onOpenExport}>
            Export
          </Button>
        ),
        renderMenuItem: () => <DropdownMenuItem onSelect={onOpenExport}>Export</DropdownMenuItem>,
      },
      {
        id: "rename",
        collapsePriority: 16,
        renderButton: () => (
          <Button type="button" variant="outline" onClick={onOpenRename}>
            Rename
          </Button>
        ),
        renderMenuItem: () => <DropdownMenuItem onSelect={onOpenRename}>Rename</DropdownMenuItem>,
      },
      {
        id: "remove",
        collapsePriority: 55,
        renderButton: () => (
          <Button type="button" variant="destructive" onClick={onOpenDelete}>
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
      onOpenCompression,
      onOpenDelete,
      onOpenDetails,
      onOpenExport,
      onOpenFilter,
      onOpenRename,
      onOpenSync,
      onOpenSwitch,
      t,
    ],
  )

  return (
    <div className="flex w-full min-w-0 items-center gap-3">
      <div className="flex min-w-[10rem] flex-1 items-center gap-2">
        <Input
          className="h-8 min-w-0 flex-1"
          value={eventSearchDraft}
          onChange={(event) => onEventSearchDraftChange(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              event.preventDefault()
              onEventSearchSubmit()
            }
          }}
          placeholder="Search events"
          data-session-event-search
        />
        <Button
          type="button"
          variant="outline"
          size="icon-sm"
          className="shrink-0"
          aria-label="Search events"
          disabled={eventSearchPending}
          onClick={onEventSearchSubmit}
          data-session-event-search-submit
        >
          {eventSearchPending ? <Spinner /> : <SearchIcon />}
        </Button>
      </div>
      <CollapsibleToolbar className="min-w-0" entries={entries} moreLabel={t("more")} />
    </div>
  )
}
