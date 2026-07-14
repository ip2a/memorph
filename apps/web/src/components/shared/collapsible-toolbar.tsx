import * as React from "react"
import { MoreHorizontalIcon } from "lucide-react"

import { Button } from "@/components/ui/button"
import { ButtonGroup } from "@/components/ui/button-group"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { cn } from "@/lib/utils"

const NAV_GAP_PX = 8

export type CollapsibleToolbarEntry = {
  id: string
  collapsePriority: number
  renderButton: () => React.ReactNode
  renderMenuItem: () => React.ReactNode
}

function ToolbarOverflowMenuContent({ entries }: { entries: CollapsibleToolbarEntry[] }) {
  return (
    <DropdownMenuContent align="end" className="w-48">
      <DropdownMenuGroup>
        {entries.map((entry) => (
          <React.Fragment key={entry.id}>{entry.renderMenuItem()}</React.Fragment>
        ))}
      </DropdownMenuGroup>
    </DropdownMenuContent>
  )
}

function ToolbarOverflowMenu({
  ariaLabel,
  entries,
}: {
  ariaLabel: string
  entries: CollapsibleToolbarEntry[]
}) {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button variant="outline" size="icon-sm" aria-label={ariaLabel}>
          <MoreHorizontalIcon />
        </Button>
      </DropdownMenuTrigger>
      <ToolbarOverflowMenuContent entries={entries} />
    </DropdownMenu>
  )
}

function ToolbarOverflowSplitButton({
  ariaLabel,
  entries,
  trailingButton,
}: {
  ariaLabel: string
  entries: CollapsibleToolbarEntry[]
  trailingButton: React.ReactNode
}) {
  return (
    <DropdownMenu>
      <ButtonGroup>
        {trailingButton}
        <DropdownMenuTrigger asChild>
          <Button variant="outline" size="icon-sm" aria-label={ariaLabel}>
            <MoreHorizontalIcon />
          </Button>
        </DropdownMenuTrigger>
      </ButtonGroup>
      <ToolbarOverflowMenuContent entries={entries} />
    </DropdownMenu>
  )
}

function useHiddenToolbarIds(
  containerRef: React.RefObject<HTMLElement | null>,
  measureRef: React.RefObject<HTMLElement | null>,
  entries: CollapsibleToolbarEntry[],
) {
  const [hiddenIds, setHiddenIds] = React.useState<Set<string>>(() => new Set())
  const entryKey = entries.map((entry) => entry.id).join("|")

  React.useLayoutEffect(() => {
    const container = containerRef.current
    const measure = measureRef.current
    if (!container || !measure) return

    const measureAndUpdate = () => {
      const measureChildren = Array.from(measure.children) as HTMLElement[]
      if (measureChildren.length < entries.length + 1) return

      const itemElements = measureChildren.slice(0, entries.length)
      const moreButton = measureChildren.at(-1)
      if (!moreButton) return

      const itemWidths = itemElements.map((element) => element.getBoundingClientRect().width)
      const moreWidth = moreButton.getBoundingClientRect().width
      const availableWidth = container.clientWidth
      const allVisibleWidth =
        itemWidths.reduce((sum, width) => sum + width, 0) +
        Math.max(0, entries.length - 1) * NAV_GAP_PX

      if (allVisibleWidth <= availableWidth) {
        setHiddenIds((current) => (current.size === 0 ? current : new Set()))
        return
      }

      const hidden = new Set<string>()

      const contentWidth = () => {
        const visibleEntries = entries.filter((entry) => !hidden.has(entry.id))
        const visibleCount = visibleEntries.length

        if (hidden.size === 0) {
          return allVisibleWidth
        }

        if (visibleCount === 0) {
          return moreWidth
        }

        const leadingEntries = visibleEntries.slice(0, -1)
        const trailingEntry = visibleEntries.at(-1)
        if (!trailingEntry) return moreWidth

        const trailingIndex = entries.findIndex((entry) => entry.id === trailingEntry.id)
        const trailingGroupWidth = itemWidths[trailingIndex] + moreWidth
        const leadingWidth = leadingEntries.reduce((sum, entry) => {
          const index = entries.findIndex((candidate) => candidate.id === entry.id)
          return sum + itemWidths[index]
        }, 0)

        if (leadingEntries.length === 0) return trailingGroupWidth
        return leadingWidth + leadingEntries.length * NAV_GAP_PX + trailingGroupWidth
      }

      while (contentWidth() > availableWidth && hidden.size < entries.length) {
        let nextHiddenId: string | null = null
        let nextPriority = -1

        for (const entry of entries) {
          if (hidden.has(entry.id) || entry.collapsePriority <= 0) continue
          if (entry.collapsePriority > nextPriority) {
            nextPriority = entry.collapsePriority
            nextHiddenId = entry.id
          }
        }

        if (!nextHiddenId) break
        hidden.add(nextHiddenId)
      }

      setHiddenIds((current) => {
        if (current.size === hidden.size && [...current].every((id) => hidden.has(id))) {
          return current
        }
        return hidden
      })
    }

    measureAndUpdate()

    const observer = new ResizeObserver(measureAndUpdate)
    observer.observe(container)

    return () => observer.disconnect()
  }, [containerRef, measureRef, entryKey, entries])

  return hiddenIds
}

export function CollapsibleToolbar({
  className,
  entries,
  moreLabel,
}: {
  className?: string
  entries: CollapsibleToolbarEntry[]
  moreLabel: string
}) {
  const slotRef = React.useRef<HTMLDivElement>(null)
  const measureRef = React.useRef<HTMLDivElement>(null)
  const hiddenIds = useHiddenToolbarIds(slotRef, measureRef, entries)
  const visibleEntries = entries.filter((entry) => !hiddenIds.has(entry.id))
  const overflowEntries = entries.filter((entry) => hiddenIds.has(entry.id))
  const hasOverflow = overflowEntries.length > 0
  const leadingVisibleEntries = hasOverflow ? visibleEntries.slice(0, -1) : visibleEntries
  const trailingVisibleEntry = hasOverflow ? visibleEntries.at(-1) : null

  if (entries.length === 0) return null

  return (
    <div ref={slotRef} className={cn("relative min-w-0", className)}>
      <div ref={measureRef} aria-hidden className="pointer-events-none invisible absolute flex gap-2">
        {entries.map((entry) => (
          <div key={entry.id}>{entry.renderButton()}</div>
        ))}
        <Button variant="outline" size="icon-sm" tabIndex={-1}>
          <MoreHorizontalIcon />
        </Button>
      </div>

      <div className="flex w-full flex-nowrap items-center justify-end gap-2 overflow-hidden">
        {leadingVisibleEntries.map((entry) => (
          <React.Fragment key={entry.id}>{entry.renderButton()}</React.Fragment>
        ))}

        {hasOverflow && trailingVisibleEntry ? (
          <ToolbarOverflowSplitButton
            ariaLabel={moreLabel}
            entries={overflowEntries}
            trailingButton={trailingVisibleEntry.renderButton()}
          />
        ) : hasOverflow ? (
          <ToolbarOverflowMenu ariaLabel={moreLabel} entries={overflowEntries} />
        ) : null}
      </div>
    </div>
  )
}
