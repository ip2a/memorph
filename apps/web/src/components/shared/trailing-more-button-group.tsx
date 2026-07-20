import type { ReactNode } from "react"
import { MoreHorizontalIcon } from "lucide-react"

import { Button } from "@/components/ui/button"
import { ButtonGroup } from "@/components/ui/button-group"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"

export function TrailingMoreButtonGroup({
  trailingAction,
  moreLabel,
  children,
}: {
  trailingAction: ReactNode
  moreLabel: string
  children: ReactNode
}) {
  return (
    <DropdownMenu>
      <ButtonGroup>
        {trailingAction}
        <DropdownMenuTrigger asChild>
          <Button type="button" variant="outline" size="icon" aria-label={moreLabel}>
            <MoreHorizontalIcon />
          </Button>
        </DropdownMenuTrigger>
      </ButtonGroup>
      <DropdownMenuContent align="end">{children}</DropdownMenuContent>
    </DropdownMenu>
  )
}
