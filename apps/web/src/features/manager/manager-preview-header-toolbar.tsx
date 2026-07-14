import * as React from "react"
import { CheckIcon, ChevronDownIcon, CopyIcon, FilterIcon, MoreHorizontalIcon, SearchIcon, Trash2Icon } from "lucide-react"

import { CollapsibleToolbar, type CollapsibleToolbarEntry } from "@/components/shared/collapsible-toolbar"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { Input } from "@/components/ui/input"
import { useI18n } from "@/lib/i18n-context"

type ManagerPreviewHeaderToolbarProps = {
  canAct: boolean
  canSelect: boolean
  canSelectFiltered: boolean
  onBackup: () => void
  onClean: () => void
  onCopyPaths: () => void
  onDeselectAll: () => void
  onInvertSelection: () => void
  onSearchChange: (value: string) => void
  onSelectAll: () => void
  onSelectFiltered: () => void
  search: string
  searchPlaceholder: string
}

export function ManagerPreviewHeaderToolbar({
  canAct,
  canSelect,
  canSelectFiltered,
  onBackup,
  onClean,
  onCopyPaths,
  onDeselectAll,
  onInvertSelection,
  onSearchChange,
  onSelectAll,
  onSelectFiltered,
  search,
  searchPlaceholder,
}: ManagerPreviewHeaderToolbarProps) {
  const { t } = useI18n()

  const entries = React.useMemo<CollapsibleToolbarEntry[]>(
    () => [
      {
        id: "clean",
        collapsePriority: 10,
        renderButton: () => (
          <Button type="button" variant="outline" size="sm" disabled={!canAct} onClick={onClean} data-manager-action-clean>
            <Trash2Icon data-icon="inline-start" />
            Clean
          </Button>
        ),
        renderMenuItem: () => (
          <DropdownMenuItem disabled={!canAct} onSelect={onClean}>
            <Trash2Icon />
            Clean
          </DropdownMenuItem>
        ),
      },
      {
        id: "backup",
        collapsePriority: 11,
        renderButton: () => (
          <Button type="button" variant="outline" size="sm" disabled={!canAct} onClick={onBackup} data-manager-action-backup>
            <CopyIcon data-icon="inline-start" />
            Backup
          </Button>
        ),
        renderMenuItem: () => (
          <DropdownMenuItem disabled={!canAct} onSelect={onBackup}>
            <CopyIcon />
            Backup
          </DropdownMenuItem>
        ),
      },
      {
        id: "more",
        collapsePriority: 40,
        renderButton: () => (
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button type="button" variant="outline" size="sm" disabled={!canAct} data-manager-action-more>
                <MoreHorizontalIcon data-icon="inline-start" />
                More
                <ChevronDownIcon data-icon="inline-end" />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end">
              <DropdownMenuItem disabled={!canAct} onSelect={onCopyPaths}>
                <CopyIcon />
                Copy Paths
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        ),
        renderMenuItem: () => (
          <DropdownMenuItem disabled={!canAct} onSelect={onCopyPaths}>
            <CopyIcon />
            Copy Paths
          </DropdownMenuItem>
        ),
      },
      {
        id: "selection",
        collapsePriority: 41,
        renderButton: () => (
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button type="button" variant="outline" size="sm" disabled={!canSelect} data-manager-selection-menu>
                <CheckIcon data-icon="inline-start" />
                Selection
                <ChevronDownIcon data-icon="inline-end" />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end">
              <DropdownMenuItem onSelect={onSelectAll}>Select All</DropdownMenuItem>
              <DropdownMenuItem onSelect={onDeselectAll}>Deselect All</DropdownMenuItem>
              <DropdownMenuItem onSelect={onInvertSelection}>Invert Selection</DropdownMenuItem>
              <DropdownMenuSeparator />
              <DropdownMenuItem disabled={!canSelectFiltered} onSelect={onSelectFiltered}>
                Select Filtered
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        ),
        renderMenuItem: () => (
          <>
            <DropdownMenuItem onSelect={onSelectAll}>Select All</DropdownMenuItem>
            <DropdownMenuItem onSelect={onDeselectAll}>Deselect All</DropdownMenuItem>
            <DropdownMenuItem onSelect={onInvertSelection}>Invert Selection</DropdownMenuItem>
            <DropdownMenuSeparator />
            <DropdownMenuItem disabled={!canSelectFiltered} onSelect={onSelectFiltered}>
              Select Filtered
            </DropdownMenuItem>
          </>
        ),
      },
      {
        id: "filters",
        collapsePriority: 42,
        renderButton: () => (
          <Button type="button" variant="outline" size="sm" disabled data-manager-preview-filters>
            <FilterIcon data-icon="inline-start" />
            Filters
          </Button>
        ),
        renderMenuItem: () => (
          <DropdownMenuItem disabled>
            <FilterIcon />
            Filters
          </DropdownMenuItem>
        ),
      },
    ],
    [
      canAct,
      canSelect,
      canSelectFiltered,
      onBackup,
      onClean,
      onCopyPaths,
      onDeselectAll,
      onInvertSelection,
      onSelectAll,
      onSelectFiltered,
    ],
  )

  return (
    <div className="flex w-full min-w-0 items-center gap-3" data-manager-preview-toolbar>
      <div className="relative w-full max-w-xs min-w-[10rem] shrink-0 md:w-52">
        <SearchIcon className="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground" aria-hidden="true" />
        <Input
          className="h-8 pl-8"
          value={search}
          onChange={(event) => onSearchChange(event.target.value)}
          placeholder={searchPlaceholder}
          data-manager-preview-search
        />
      </div>
      <CollapsibleToolbar className="min-w-0 flex-1" entries={entries} moreLabel={t("more")} />
    </div>
  )
}
