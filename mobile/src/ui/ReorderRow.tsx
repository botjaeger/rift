import type { ReactNode } from "react";
import { useEffect, useState } from "react";
import { Animated, PanResponder, type AccessibilityActionEvent } from "react-native";
import { shiftOf, slotAt, stepTo } from "@/reorder";
import { colors } from "./theme";

/** Which row is being dragged and over which place (React state: the other rows slide to make room). */
export interface Dragging {
  active: number | null;
  slot: number;
}

/** The list's drag state; `scrollEnabled` goes on the sheet's ScrollView so a drag never scrolls it. */
export function useReorder(): { drag: Dragging; setDrag: (d: Dragging) => void; scrollEnabled: boolean } {
  const [drag, setDrag] = useState<Dragging>({ active: null, slot: 0 });
  return { drag, setDrag, scrollEnabled: drag.active === null };
}

/** What the row's pressable takes: the long press that lifts the row, and Move up / Move down for a screen reader. */
export interface RowHandle {
  onLongPress: () => void;
  onPressOut: () => void;
  accessibilityActions: { name: string; label: string }[];
  onAccessibilityAction: (e: AccessibilityActionEvent) => void;
}

/** The row's drag, made once: handlers read the latest props through `latest`, never while rendering. */
function makeRow(first: Props) {
  let current = first;
  const latest = () => current;
  const dy = new Animated.Value(0);
  const s = { armed: false, captured: false };
  const reset = () => {
    s.armed = false;
    s.captured = false;
    dy.setValue(0);
    latest().setDrag({ active: null, slot: 0 });
  };
  const pan = PanResponder.create({
    // after the long press, the next movement is the drag: take it from the pressable under the finger
    onMoveShouldSetPanResponderCapture: () => {
      if (s.armed) s.captured = true;
      return s.armed;
    },
    onPanResponderTerminationRequest: () => false,
    onPanResponderMove: (_, g) => {
      const p = latest();
      const lo = -p.index * p.rowH;
      const hi = (p.count - 1 - p.index) * p.rowH;
      dy.setValue(Math.min(hi, Math.max(lo, g.dy)));
      const slot = slotAt(p.index, g.dy, p.rowH, p.count);
      if (slot !== p.drag.slot) p.setDrag({ active: p.index, slot });
    },
    onPanResponderRelease: (_, g) => {
      const p = latest();
      const slot = slotAt(p.index, g.dy, p.rowH, p.count);
      reset();
      if (slot !== p.index) p.onMove(p.index, slot);
    },
    onPanResponderTerminate: reset,
  });
  const handle = (p: Props): RowHandle => {
    const up = stepTo(p.index, -1, p.count);
    const down = stepTo(p.index, 1, p.count);
    return {
      onLongPress: () => {
        s.armed = true;
        latest().setDrag({ active: latest().index, slot: latest().index });
      },
      // a long press without a drag ends here; a drag ends in the pan responder
      onPressOut: () => {
        if (s.armed && !s.captured) reset();
      },
      accessibilityActions: [
        ...(up === null ? [] : [{ name: "moveUp", label: "Move up" }]),
        ...(down === null ? [] : [{ name: "moveDown", label: "Move down" }]),
      ],
      onAccessibilityAction: (e) => {
        const q = latest();
        const to = stepTo(q.index, e.nativeEvent.actionName === "moveUp" ? -1 : 1, q.count);
        if (to !== null && (e.nativeEvent.actionName === "moveUp" || e.nativeEvent.actionName === "moveDown")) q.onMove(q.index, to);
      },
    };
  };
  return { dy, pan, handle, update: (p: Props) => (current = p) };
}

interface Props {
  index: number;
  count: number;
  /** The distance between this row's top and the next one's (its height and the list's gap). */
  rowH: number;
  drag: Dragging;
  setDrag: (d: Dragging) => void;
  /** Row `from` was dropped on place `to`. */
  onMove: (from: number, to: number) => void;
  /** Called with the pressable's props. */
  children: (handle: RowHandle) => ReactNode;
}

/**
 * One row of a list that reorders: a long press lifts it, dragging moves it over the others (they
 * slide aside), letting go drops it. The same move is a pair of accessibility actions, "Move up" and
 * "Move down", for a screen reader. Rows are `rowH` apart.
 */
export function ReorderRow(props: Props) {
  const [row] = useState(() => makeRow(props));
  useEffect(() => {
    row.update(props);
  });
  const { index, rowH, drag } = props;
  const lifted = drag.active === index;
  const shift = drag.active === null ? 0 : shiftOf(index, drag.active, drag.slot, rowH);
  return (
    <Animated.View
      {...row.pan.panHandlers}
      style={[
        lifted ? { zIndex: 2, elevation: 6, shadowColor: colors.bg, shadowOpacity: 0.5, shadowRadius: 8, transform: [{ translateY: row.dy }] } : { transform: [{ translateY: shift }] },
      ]}
    >
      {props.children(row.handle(props))}
    </Animated.View>
  );
}
