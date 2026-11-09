import { GripVertical } from "lucide-react";
import { cn } from "@/lib/utils";

/**
 * Where a drag ended: the `data-reorder-list` container under the pointer
 * and, when the pointer was over one, its `data-reorder-item` value.
 */
export type ReorderDropTarget = { list: string; item: string | null };

/**
 * A grip that drags with pointer capture and reports the list and item under
 * the pointer on release. Lists mark themselves with `data-reorder-list` and
 * their rows with `data-reorder-item`. The grip is a pointer convenience;
 * every list that uses it also offers buttons for the same moves.
 */
export function PointerReorderHandle({
  id,
  title,
  onDrop,
  className,
}: {
  id?: string;
  title: string;
  onDrop: (target: ReorderDropTarget) => void;
  className?: string;
}) {
  return (
    <span
      id={id}
      className={cn("cursor-grab touch-none active:cursor-grabbing", className)}
      title={title}
      onPointerDown={(event) => {
        if (event.button !== 0) return;
        event.preventDefault();
        event.currentTarget.setPointerCapture(event.pointerId);
      }}
      onPointerUp={(event) => {
        if (!event.currentTarget.hasPointerCapture(event.pointerId)) return;
        event.currentTarget.releasePointerCapture(event.pointerId);
        const under = document.elementFromPoint(event.clientX, event.clientY);
        const list = under?.closest<HTMLElement>("[data-reorder-list]");
        const listId = list?.dataset.reorderList;
        if (!list || !listId) return;
        const item = under?.closest<HTMLElement>("[data-reorder-item]");
        onDrop({
          list: listId,
          item:
            item && list.contains(item)
              ? (item.dataset.reorderItem ?? null)
              : null,
        });
      }}
    >
      <GripVertical className="h-4 w-4" aria-hidden="true" />
    </span>
  );
}
