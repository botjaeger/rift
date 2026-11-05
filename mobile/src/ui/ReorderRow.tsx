import type { ReactNode } from "react";
import { useEffect, useMemo, useState } from "react";
import { Animated, type AccessibilityActionEvent } from "react-native";
import { Gesture, GestureDetector } from "react-native-gesture-handler";
import { dragLift } from "@/haptics";
import { shiftOf, slotAt, stepTo } from "@/reorder";
import { colors } from "./theme";

/** How long a press holds before the row lifts (the platform's long press). */
const LIFT_MS = 400;

/** Which row is being dragged and over which place (React state: the other rows slide to make room). */
export interface Dragging {
  active: number | null;
  slot: number;
}

/** One list's drag state; `scrollEnabled` goes on the page's ScrollView so a drag never scrolls it. */
export function useReorder(): { drag: Dragging; setDrag: (d: Dragging) => void; scrollEnabled: boolean } {
  const [drag, setDrag] = useState<Dragging>({ active: null, slot: 0 });
  return { drag, setDrag, scrollEnabled: drag.active === null };
}

/** What the row's pressable takes: Move up / Move down for a screen reader. */
export interface RowHandle {
  accessibilityActions: { name: string; label: string }[];
  onAccessibilityAction: (e: AccessibilityActionEvent) => void;
  /** With `grip`: wraps the row's grip, which drags the row at once (the rest of the row stays a button). */
  grip: (node: ReactNode) => ReactNode;
}

/** The row's drag, made once: handlers read the latest props through `latest`, never while rendering. */
function makeRow(first: Props) {
  // Every handler's state lives in this one object, mutated, never in reassigned variables: the
  // worklets Babel plugin turns gesture callbacks (inline or named) into worklets holding their
  // own copy of each variable they capture, even with runOnJS. `lifted` set in onStart was then
  // still false in onEnd and a drop moved nothing; `latest()` read the props of the first render.
  // A captured object is shared on the JS thread, so its properties stay live.
  const box = { props: first, lifted: false };
  const latest = () => box.props;
  const dy = new Animated.Value(0);
  const reset = () => {
    box.lifted = false;
    dy.setValue(0);
    latest().setDrag({ active: null, slot: 0 });
  };
  function onStart() {
    box.lifted = true;
    dragLift();
    const p = latest();
    p.setDrag({ active: p.index, slot: p.index });
  }
  function onUpdate(e: { translationY: number }) {
    if (!box.lifted) return;
    const p = latest();
    const lo = -p.index * p.rowH;
    const hi = (p.count - 1 - p.index) * p.rowH;
    dy.setValue(Math.min(hi, Math.max(lo, e.translationY)));
    const slot = slotAt(p.index, e.translationY, p.rowH, p.count);
    if (slot !== p.drag.slot) p.setDrag({ active: p.index, slot });
  }
  function onEnd(e: { translationY: number }) {
    if (!box.lifted) return;
    const p = latest();
    const slot = slotAt(p.index, e.translationY, p.rowH, p.count);
    reset();
    if (slot !== p.index) p.onMove(p.index, slot);
  }
  // a cancelled drag (the page closed, another gesture won) puts the row back
  function onFinalize() {
    if (box.lifted) reset();
  }
  // a grip lifts the row on its first movement; a whole row lifts after a long press, so a swipe
  // over it still scrolls the list
  const gesture = (enabled: boolean, grip: boolean) => {
    const pan = Gesture.Pan().runOnJS(true).enabled(enabled);
    return (grip ? pan.minDistance(1) : pan.activateAfterLongPress(LIFT_MS)).onStart(onStart).onUpdate(onUpdate).onEnd(onEnd).onFinalize(onFinalize);
  };
  const handle = (p: Props, grip: (node: ReactNode) => ReactNode): RowHandle => {
    const up = stepTo(p.index, -1, p.count);
    const down = stepTo(p.index, 1, p.count);
    return {
      grip,
      accessibilityActions: p.enabled
        ? [...(up === null ? [] : [{ name: "moveUp", label: "Move up" }]), ...(down === null ? [] : [{ name: "moveDown", label: "Move down" }])]
        : [],
      onAccessibilityAction: (e) => {
        const name = e.nativeEvent.actionName;
        if (name !== "moveUp" && name !== "moveDown") return;
        const q = latest();
        const to = stepTo(q.index, name === "moveUp" ? -1 : 1, q.count);
        if (to !== null) q.onMove(q.index, to);
      },
    };
  };
  return { dy, gesture, handle, update: (p: Props) => (box.props = p) };
}

interface Props {
  index: number;
  count: number;
  /** The distance between this row's top and the next one's (its height and the list's gap). */
  rowH: number;
  /** Whether the row can be dragged at all. */
  enabled: boolean;
  drag: Dragging;
  setDrag: (d: Dragging) => void;
  /** Row `from` was dropped on place `to`. */
  onMove: (from: number, to: number) => void;
  /** The row drags by its grip (`handle.grip`), at once, rather than by a long press on all of it. */
  grip?: boolean;
  /** Called with the pressable's props. */
  children: (handle: RowHandle) => ReactNode;
}

/**
 * One row of a list that reorders: a long press lifts it, dragging moves it over the others (they
 * slide aside), letting go drops it. The same move is a pair of accessibility actions, "Move up" and
 * "Move down", for a screen reader. Rows are `rowH` apart.
 *
 * The drag is a native pan (gesture handler), not a PanResponder: when these lists were form sheets
 * the sheet's own pan took every movement and cancelled the JS touch; they are full pages now, and
 * once this pan activates (after the long press) it still prevents the scroll view's.
 */
export function ReorderRow(props: Props) {
  const [row] = useState(() => makeRow(props));
  useEffect(() => {
    row.update(props);
  });
  const { index, rowH, drag, enabled } = props;
  const grip = !!props.grip;
  const pan = useMemo(() => row.gesture(enabled, grip), [row, enabled, grip]);
  const lifted = drag.active === index;
  const shift = drag.active === null ? 0 : shiftOf(index, drag.active, drag.slot, rowH);
  const wrapGrip = (node: ReactNode) => (grip ? <GestureDetector gesture={pan}>{node}</GestureDetector> : node);
  // zIndex on both sides: a row that got one only when lifted remounted under the finger (iOS ended the drag)
  const body = (
    <Animated.View
      style={[
        lifted ? { zIndex: 2, elevation: 6, shadowColor: colors.bg, shadowOpacity: 0.5, shadowRadius: 8, transform: [{ translateY: row.dy }] } : { zIndex: 0, transform: [{ translateY: shift }] },
      ]}
    >
      {props.children(row.handle(props, wrapGrip))}
    </Animated.View>
  );
  return grip ? body : <GestureDetector gesture={pan}>{body}</GestureDetector>;
}
