import * as React from "react";
import { GripVertical } from "lucide-react";
import { cn } from "@/lib/utils";

/**
 * Where a drag ended: the `data-reorder-list` container under the pointer
 * and, when the pointer was over one, its `data-reorder-item` value.
 */
export type ReorderDropTarget = { list: string; item: string | null };

/** How far the pointer travels before a press becomes a drag, in pixels. */
const DRAG_THRESHOLD_PX = 3;
const SHIFT_MS = 180;
const SETTLE_MS = 160;
const EASING = "cubic-bezier(0.2, 0.8, 0.2, 1)";

type Sibling = { element: HTMLElement; top: number; height: number };

/** Another ordered list the row hovers over, opened at `slot` for it. */
type Foreign = {
  list: HTMLElement;
  siblings: Sibling[];
  gap: number;
  slot: number;
  paddingBottom: string;
};

type Drag = {
  pointerId: number;
  startX: number;
  startY: number;
  row: HTMLElement | null;
  list: HTMLElement | null;
  listId: string | null;
  /** Rows of the list in order, measured relative to the list at the start. */
  siblings: Sibling[];
  from: number;
  gap: number;
  /** Where the dragged row would land if dropped now, in the same list. */
  to: number;
  lifted: boolean;
  clone: HTMLElement | null;
  grabX: number;
  grabY: number;
  rowStyle: string | null;
  childVisibility: Array<[HTMLElement, string]>;
  hoveredList: HTMLElement | null;
  foreign: Foreign | null;
  /** The dragged row's height, for the slot it opens in another list. */
  height: number;
  reducedMotion: boolean;
};

function rowsOf(list: HTMLElement): HTMLElement[] {
  return Array.from(list.children).filter(
    (child): child is HTMLElement =>
      child instanceof HTMLElement && child.dataset.reorderItem !== undefined,
  );
}

/** The list's rows, measured from the top of its scrolled content. */
function measure(list: HTMLElement): { siblings: Sibling[]; gap: number } {
  const listTop = list.getBoundingClientRect().top - list.scrollTop;
  const siblings = rowsOf(list).map((element) => {
    const r = element.getBoundingClientRect();
    return { element, top: r.top - listTop, height: r.height };
  });
  const [first, second] = siblings;
  const gap =
    first && second ? Math.max(0, second.top - (first.top + first.height)) : 0;
  return { siblings, gap };
}

function contentY(list: HTMLElement, clientY: number): number {
  return clientY - list.getBoundingClientRect().top + list.scrollTop;
}

function stripIds(element: HTMLElement) {
  element.removeAttribute("id");
  for (const child of element.querySelectorAll("[id]")) {
    child.removeAttribute("id");
  }
}

/**
 * How far each row moves to open the slot at `to` for the row dragged from
 * `from`: rows between the two shift by the dragged row's height, and the
 * dragged row's own ghost moves into the slot.
 */
function shiftFor(drag: Drag, index: number): number {
  const { from, to, siblings, gap } = drag;
  const dragged = siblings[from];
  if (index === from) {
    if (to > from) {
      let offset = 0;
      for (let i = from + 1; i <= to; i += 1)
        offset += siblings[i].height + gap;
      return offset;
    }
    let offset = 0;
    for (let i = to; i < from; i += 1) offset -= siblings[i].height + gap;
    return offset;
  }
  if (to > from && index > from && index <= to) return -(dragged.height + gap);
  if (to < from && index >= to && index < from) return dragged.height + gap;
  return 0;
}

/** The slot the pointer is over: how many other rows sit above it. */
function slotAt(drag: Drag, clientY: number): number {
  if (!drag.list) return drag.from;
  const y = contentY(drag.list, clientY);
  let slot = 0;
  drag.siblings.forEach((sibling, index) => {
    if (index === drag.from) return;
    if (sibling.top + sibling.height / 2 < y) slot += 1;
  });
  return slot;
}

function applyShifts(drag: Drag) {
  drag.siblings.forEach((sibling, index) => {
    const offset = shiftFor(drag, index);
    sibling.element.style.transform = offset ? `translateY(${offset}px)` : "";
  });
}

function transitionRows(drag: Drag, siblings: Sibling[]) {
  for (const sibling of siblings) {
    sibling.element.style.transition = drag.reducedMotion
      ? ""
      : `transform ${SHIFT_MS}ms ${EASING}`;
    sibling.element.style.willChange = "transform";
  }
}

function clearRows(siblings: Sibling[], animated: boolean) {
  for (const sibling of siblings) {
    if (!animated) sibling.element.style.transition = "";
    sibling.element.style.transform = "";
    sibling.element.style.willChange = "";
  }
}

/**
 * Over another ordered list, the rows from the pointer's slot down slide
 * aside to open a slot for the row; leaving it closes the slot again. An
 * unordered list (`data-reorder-unordered`) is only outlined.
 */
function updateForeign(drag: Drag, list: HTMLElement | null, clientY: number) {
  const ordered =
    list && list !== drag.list && list.dataset.reorderUnordered === undefined
      ? list
      : null;
  if (drag.foreign && drag.foreign.list !== ordered) {
    closeForeign(drag, true);
  }
  if (!ordered) return;
  if (!drag.foreign) {
    const { siblings, gap } = measure(ordered);
    drag.foreign = {
      list: ordered,
      siblings,
      gap: gap || drag.gap || 6,
      slot: -1,
      paddingBottom: ordered.style.paddingBottom,
    };
    transitionRows(drag, siblings);
    // The list grows by the opened slot, so its last row is never cut off.
    const padding = parseFloat(getComputedStyle(ordered).paddingBottom) || 0;
    ordered.style.transition = drag.reducedMotion
      ? ""
      : `padding-bottom ${SHIFT_MS}ms ${EASING}`;
    ordered.style.paddingBottom = `${padding + drag.height + drag.foreign.gap}px`;
  }
  const foreign = drag.foreign;
  const y = contentY(ordered, clientY);
  let slot = 0;
  for (const sibling of foreign.siblings) {
    if (sibling.top + sibling.height / 2 < y) slot += 1;
  }
  if (slot === foreign.slot) return;
  foreign.slot = slot;
  foreign.siblings.forEach((sibling, index) => {
    sibling.element.style.transform =
      index >= slot ? `translateY(${drag.height + foreign.gap}px)` : "";
  });
}

function closeForeign(drag: Drag, animated: boolean) {
  const foreign = drag.foreign;
  if (!foreign) return;
  clearRows(foreign.siblings, animated);
  if (!animated) foreign.list.style.transition = "";
  foreign.list.style.paddingBottom = foreign.paddingBottom;
  drag.foreign = null;
}

function setHoveredList(drag: Drag, list: HTMLElement | null) {
  if (drag.hoveredList === list) return;
  if (drag.hoveredList) {
    drag.hoveredList.style.outline = "";
    drag.hoveredList.style.outlineOffset = "";
  }
  drag.hoveredList = list;
  if (list) {
    list.style.outline = "2px dashed rgba(var(--scry-accent-rgb), 0.55)";
    list.style.outlineOffset = "2px";
  }
}

function lift(drag: Drag, clientX: number, clientY: number) {
  const { row, list } = drag;
  drag.lifted = true;
  document.body.style.cursor = "grabbing";
  document.body.style.userSelect = "none";
  if (!row) return;
  const rect = row.getBoundingClientRect();
  drag.height = rect.height;
  drag.grabX = drag.startX - rect.left;
  drag.grabY = drag.startY - rect.top;

  const clone = row.cloneNode(true) as HTMLElement;
  stripIds(clone);
  clone.setAttribute("aria-hidden", "true");
  Object.assign(clone.style, {
    position: "fixed",
    left: `${rect.left}px`,
    top: `${rect.top}px`,
    width: `${rect.width}px`,
    height: `${rect.height}px`,
    margin: "0",
    zIndex: "1000",
    pointerEvents: "none",
    listStyle: "none",
    boxSizing: "border-box",
    boxShadow: "0 12px 32px rgba(0, 0, 0, 0.45)",
    transform: "scale(1.02)",
    transition: drag.reducedMotion ? "" : `transform 120ms ${EASING}`,
  });
  document.body.appendChild(clone);
  drag.clone = clone;
  moveClone(drag, clientX, clientY);

  // The row stays in the list as a ghost: an empty dashed box holding the
  // slot the row will drop into.
  drag.rowStyle = row.getAttribute("style");
  row.style.borderStyle = "dashed";
  row.style.borderColor = "rgba(var(--scry-accent-rgb), 0.6)";
  row.style.background = "rgba(var(--scry-accent-rgb), 0.08)";
  row.style.boxShadow = "none";
  for (const child of Array.from(row.children)) {
    if (child instanceof HTMLElement) {
      drag.childVisibility.push([child, child.style.visibility]);
      child.style.visibility = "hidden";
    }
  }

  if (list) {
    const { siblings, gap } = measure(list);
    drag.siblings = siblings;
    drag.gap = gap;
    drag.from = siblings.findIndex((s) => s.element === row);
    drag.to = drag.from;
    transitionRows(drag, siblings);
  }
}

function moveClone(drag: Drag, clientX: number, clientY: number) {
  if (!drag.clone) return;
  drag.clone.style.left = `${clientX - drag.grabX}px`;
  drag.clone.style.top = `${clientY - drag.grabY}px`;
}

/** Puts every row and the page back as they were before the drag. */
function restore(drag: Drag) {
  drag.clone?.remove();
  drag.clone = null;
  setHoveredList(drag, null);
  closeForeign(drag, false);
  clearRows(drag.siblings, false);
  if (drag.row) {
    if (drag.rowStyle === null) drag.row.removeAttribute("style");
    else drag.row.setAttribute("style", drag.rowStyle);
  }
  for (const [child, visibility] of drag.childVisibility) {
    child.style.visibility = visibility;
  }
  drag.childVisibility = [];
  document.body.style.cursor = "";
  document.body.style.userSelect = "";
}

/** Where the slot opened in another list sits on screen. */
function foreignSlotRect(
  foreign: Foreign,
): { left: number; top: number } | null {
  const listRect = foreign.list.getBoundingClientRect();
  const origin = listRect.top - foreign.list.scrollTop;
  const style = getComputedStyle(foreign.list);
  const left =
    listRect.left +
    (parseFloat(style.borderLeftWidth) || 0) +
    (parseFloat(style.paddingLeft) || 0);
  const at = foreign.siblings[foreign.slot];
  if (at) return { left, top: origin + at.top };
  const last = foreign.siblings[foreign.siblings.length - 1];
  if (last) return { left, top: origin + last.top + last.height + foreign.gap };
  return {
    left,
    top:
      listRect.top +
      (parseFloat(style.borderTopWidth) || 0) +
      (parseFloat(style.paddingTop) || 0),
  };
}

function dropTargetAt(
  clientX: number,
  clientY: number,
  drag: Drag,
): ReorderDropTarget | null {
  // The floating copy ignores the pointer, so this finds what lies beneath.
  const under = document.elementFromPoint(clientX, clientY);
  const list = under?.closest<HTMLElement>("[data-reorder-list]");
  const listId = list?.dataset.reorderList;
  if (!list || !listId) return null;
  if (list === drag.list && drag.siblings.length > 0 && drag.from >= 0) {
    // In its own list the row lands where its ghost shows it, which is the
    // slot of the row now holding that position.
    return {
      list: listId,
      item: drag.siblings[drag.to].element.dataset.reorderItem ?? null,
    };
  }
  if (drag.foreign?.list === list) {
    // In another ordered list the row lands in the slot opened for it,
    // before the row that held that slot.
    return {
      list: listId,
      item:
        drag.foreign.siblings[drag.foreign.slot]?.element.dataset.reorderItem ??
        null,
    };
  }
  const item = under?.closest<HTMLElement>("[data-reorder-item]");
  return {
    list: listId,
    item:
      item && list.contains(item) ? (item.dataset.reorderItem ?? null) : null,
  };
}

/** Controls inside a row that keep their own press instead of starting a drag. */
const INTERACTIVE =
  "button, a, input, select, textarea, label, [role='button'], [contenteditable='true']";

/**
 * A grip that drags its row with pointer capture. The whole row is the grab
 * area with a mouse, except its buttons and other controls; on touch only the
 * grip drags, so the rest of the row still scrolls the list. While dragging,
 * the row follows the pointer, its place in the list shows as an empty ghost
 * box, and the other rows slide aside to show where it will land. On release
 * it reports the list and item under the pointer. Lists mark themselves with
 * `data-reorder-list` and their rows with `data-reorder-item`. Dragging is a
 * pointer convenience; every list that uses it also offers buttons for the
 * same moves.
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
  const gripRef = React.useRef<HTMLSpanElement | null>(null);
  const dragRef = React.useRef<Drag | null>(null);
  const settleRef = React.useRef<number | null>(null);
  const onDropRef = React.useRef(onDrop);
  React.useLayoutEffect(() => {
    onDropRef.current = onDrop;
  });

  React.useEffect(() => {
    const grip = gripRef.current;
    if (!grip) return;
    const row =
      grip.closest<HTMLElement>("[data-reorder-item]") ??
      grip.closest<HTMLElement>("li");
    const surface: HTMLElement = row ?? grip;
    if (row) row.dataset.reorderGrab = "";

    const onPointerDown = (event: PointerEvent) => {
      if (event.button !== 0 || dragRef.current) return;
      const target = event.target as Element | null;
      const onGrip = !!target && grip.contains(target);
      if (!onGrip) {
        // Away from the grip the row drags only with a mouse or pen, and
        // never from one of its controls.
        if (event.pointerType === "touch") return;
        if (target?.closest(INTERACTIVE)) return;
      }
      event.preventDefault();
      surface.setPointerCapture(event.pointerId);
      const list = row?.parentElement?.closest<HTMLElement>(
        "[data-reorder-list]",
      );
      dragRef.current = {
        pointerId: event.pointerId,
        startX: event.clientX,
        startY: event.clientY,
        row: row ?? null,
        list: list && row?.dataset.reorderItem !== undefined ? list : null,
        listId: list?.dataset.reorderList ?? null,
        siblings: [],
        from: -1,
        gap: 0,
        to: -1,
        lifted: false,
        clone: null,
        grabX: 0,
        grabY: 0,
        rowStyle: null,
        childVisibility: [],
        hoveredList: null,
        foreign: null,
        height: 0,
        reducedMotion:
          window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ??
          false,
      };
    };

    const onPointerMove = (event: PointerEvent) => {
      const drag = dragRef.current;
      if (!drag || drag.pointerId !== event.pointerId) return;
      if (!drag.lifted) {
        if (
          Math.hypot(event.clientX - drag.startX, event.clientY - drag.startY) <
          DRAG_THRESHOLD_PX
        )
          return;
        lift(drag, event.clientX, event.clientY);
      }
      moveClone(drag, event.clientX, event.clientY);
      const under = document.elementFromPoint(event.clientX, event.clientY);
      const overList = under?.closest<HTMLElement>("[data-reorder-list]");
      if (drag.list && drag.from >= 0) {
        // Over another list the row leaves this one, so its slot stays put.
        const to =
          overList && overList !== drag.list
            ? drag.from
            : slotAt(drag, event.clientY);
        if (to !== drag.to) {
          drag.to = to;
          applyShifts(drag);
        }
      }
      setHoveredList(
        drag,
        overList && overList !== drag.list ? overList : null,
      );
      updateForeign(drag, overList ?? null, event.clientY);
    };

    const onPointerUp = (event: PointerEvent) => {
      const drag = dragRef.current;
      if (!drag || drag.pointerId !== event.pointerId) return;
      if (surface.hasPointerCapture(event.pointerId)) {
        surface.releasePointerCapture(event.pointerId);
      }
      if (!drag.lifted) {
        // A press that never moved is a click, not a drop.
        dragRef.current = null;
        return;
      }
      const target = dropTargetAt(event.clientX, event.clientY, drag);
      const finish = () => {
        settleRef.current = null;
        restore(drag);
        dragRef.current = null;
        if (target) onDropRef.current(target);
      };
      // The floating row glides into its ghost's slot before the list
      // takes its new order.
      const ghostRect =
        !drag.clone || drag.reducedMotion || !target
          ? null
          : target.list === drag.listId && drag.row
            ? drag.row.getBoundingClientRect()
            : drag.foreign?.list.dataset.reorderList === target.list
              ? foreignSlotRect(drag.foreign)
              : null;
      if (drag.clone && ghostRect) {
        Object.assign(drag.clone.style, {
          transition: `left ${SETTLE_MS}ms ${EASING}, top ${SETTLE_MS}ms ${EASING}, transform ${SETTLE_MS}ms ${EASING}, box-shadow ${SETTLE_MS}ms ${EASING}`,
          left: `${ghostRect.left}px`,
          top: `${ghostRect.top}px`,
          transform: "scale(1)",
          boxShadow: "0 0 0 rgba(0, 0, 0, 0)",
        });
        settleRef.current = window.setTimeout(finish, SETTLE_MS);
      } else {
        finish();
      }
    };

    const onPointerCancel = (event: PointerEvent) => {
      const drag = dragRef.current;
      if (!drag || drag.pointerId !== event.pointerId) return;
      restore(drag);
      dragRef.current = null;
    };

    surface.addEventListener("pointerdown", onPointerDown);
    surface.addEventListener("pointermove", onPointerMove);
    surface.addEventListener("pointerup", onPointerUp);
    surface.addEventListener("pointercancel", onPointerCancel);
    return () => {
      surface.removeEventListener("pointerdown", onPointerDown);
      surface.removeEventListener("pointermove", onPointerMove);
      surface.removeEventListener("pointerup", onPointerUp);
      surface.removeEventListener("pointercancel", onPointerCancel);
      if (row) delete row.dataset.reorderGrab;
      if (settleRef.current !== null) window.clearTimeout(settleRef.current);
      settleRef.current = null;
      if (dragRef.current) restore(dragRef.current);
      dragRef.current = null;
    };
  }, []);

  return (
    <span
      ref={gripRef}
      id={id}
      className={cn("cursor-grab touch-none active:cursor-grabbing", className)}
      title={title}
    >
      <GripVertical className="h-4 w-4" aria-hidden="true" />
    </span>
  );
}
