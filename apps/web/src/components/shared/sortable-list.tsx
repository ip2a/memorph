import {
  DndContext,
  DragOverlay,
  KeyboardSensor,
  PointerSensor,
  closestCenter,
  useSensor,
  useSensors,
  type DragEndEvent,
  type DragStartEvent,
} from "@dnd-kit/core";
import {
  SortableContext,
  arrayMove,
  sortableKeyboardCoordinates,
  useSortable,
  verticalListSortingStrategy,
} from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import { GripVerticalIcon } from "lucide-react";
import { createContext, useContext, useState, type CSSProperties, type ReactNode } from "react";
import { buttonVariants } from "@/components/ui/button";
import { cn } from "@/lib/utils";

type SortableItemContextValue = {
  attributes: ReturnType<typeof useSortable>["attributes"];
  listeners: ReturnType<typeof useSortable>["listeners"];
  setActivatorNodeRef: ReturnType<typeof useSortable>["setActivatorNodeRef"];
  isDragging: boolean;
};

const SortableItemContext = createContext<SortableItemContextValue | null>(null);

function useSortableItemContext() {
  const value = useContext(SortableItemContext);
  if (!value) throw new Error("SortableItemHandle must be used within SortableItem");
  return value;
}

type SortableListProps<T extends { id: string }> = {
  items: T[];
  onReorder: (items: T[]) => void;
  className?: string;
  children: ReactNode;
  renderOverlay?: (item: T) => ReactNode;
};

export function SortableList<T extends { id: string }>({
  items,
  onReorder,
  className,
  children,
  renderOverlay,
}: SortableListProps<T>) {
  const [activeId, setActiveId] = useState<string | null>(null);
  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 6 } }),
    useSensor(KeyboardSensor, { coordinateGetter: sortableKeyboardCoordinates }),
  );
  const ids = items.map((item) => item.id);
  const activeItem = activeId ? items.find((item) => item.id === activeId) : undefined;

  function handleDragStart(event: DragStartEvent) {
    setActiveId(String(event.active.id));
  }

  function handleDragEnd(event: DragEndEvent) {
    const { active, over } = event;
    setActiveId(null);
    if (!over || active.id === over.id) return;
    const oldIndex = ids.indexOf(String(active.id));
    const newIndex = ids.indexOf(String(over.id));
    if (oldIndex < 0 || newIndex < 0) return;
    onReorder(arrayMove(items, oldIndex, newIndex));
  }

  return (
    <DndContext
      sensors={sensors}
      collisionDetection={closestCenter}
      onDragStart={handleDragStart}
      onDragEnd={handleDragEnd}
      onDragCancel={() => setActiveId(null)}
    >
      <SortableContext items={ids} strategy={verticalListSortingStrategy}>
        <div className={className}>{children}</div>
      </SortableContext>
      <DragOverlay dropAnimation={null}>
        {activeItem && renderOverlay ? (
          <div className="cursor-grabbing rounded-md border bg-background shadow-md">{renderOverlay(activeItem)}</div>
        ) : null}
      </DragOverlay>
    </DndContext>
  );
}

type SortableItemProps = {
  id: string;
  className?: string;
  style?: CSSProperties;
  children: ReactNode;
};

export function SortableItem({ id, className, style, children }: SortableItemProps) {
  const { attributes, listeners, setNodeRef, setActivatorNodeRef, transform, transition, isDragging } = useSortable({ id });
  const itemStyle: CSSProperties = {
    ...style,
    transform: CSS.Transform.toString(transform),
    transition,
  };

  return (
    <SortableItemContext.Provider value={{ attributes, listeners, setActivatorNodeRef, isDragging }}>
      <div
        ref={setNodeRef}
        style={itemStyle}
        className={cn(className, isDragging ? "z-10 opacity-50" : "")}
        data-sortable-item={id}
      >
        {children}
      </div>
    </SortableItemContext.Provider>
  );
}

type SortableItemHandleProps = {
  label: string;
  className?: string;
};

export function SortableItemHandle({ label, className }: SortableItemHandleProps) {
  const { attributes, listeners, setActivatorNodeRef } = useSortableItemContext();

  return (
    <button
      ref={setActivatorNodeRef}
      type="button"
      className={cn(
        buttonVariants({ variant: "ghost", size: "icon-xs" }),
        "cursor-grab text-muted-foreground active:cursor-grabbing",
        className,
      )}
      aria-label={label}
      {...attributes}
      {...listeners}
    >
      <GripVerticalIcon />
    </button>
  );
}
