import { useState, type ReactNode } from "react"
import { Link } from "react-router-dom"
import { MoreHorizontalIcon } from "lucide-react"

import { Button } from "@/components/ui/button"
import { ButtonGroup } from "@/components/ui/button-group"
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"

export type TrailingMoreAction = {
  id: string
  label: string
  variant?: "outline" | "destructive"
  href?: string
  onSelect?: () => void
}

function ActionButton({
  action,
  onActivate,
}: {
  action: TrailingMoreAction
  onActivate?: () => void
}) {
  const variant = action.variant ?? "outline"

  if (action.href) {
    return (
      <Button asChild variant={variant} className="justify-center" onClick={onActivate}>
        <Link to={action.href}>{action.label}</Link>
      </Button>
    )
  }

  return (
    <Button
      type="button"
      variant={variant}
      className="justify-center"
      onClick={() => {
        onActivate?.()
        action.onSelect?.()
      }}
    >
      {action.label}
    </Button>
  )
}

export function TrailingMoreActionsDialog({
  trailingAction,
  moreLabel,
  dialogTitle,
  actions,
}: {
  trailingAction: ReactNode
  moreLabel: string
  dialogTitle: string
  actions: TrailingMoreAction[]
}) {
  const [open, setOpen] = useState(false)

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <ButtonGroup>
        {trailingAction}
        <Button
          type="button"
          variant="outline"
          size="icon"
          aria-label={moreLabel}
          onClick={() => setOpen(true)}
        >
          <MoreHorizontalIcon />
        </Button>
      </ButtonGroup>
      <DialogContent className="sm:max-w-md" data-trailing-more-actions-dialog>
        <DialogHeader>
          <DialogTitle>{dialogTitle}</DialogTitle>
        </DialogHeader>
        <div className="grid grid-cols-2 gap-2 sm:grid-cols-3">
          {actions.map((action) => (
            <ActionButton key={action.id} action={action} onActivate={() => setOpen(false)} />
          ))}
        </div>
      </DialogContent>
    </Dialog>
  )
}
