import { createContext, useCallback, useContext, useMemo, useRef, useState, type ReactNode, useImperativeHandle, type Ref } from "react";
import { AccessibilityInfo, Platform, StyleSheet, View, type GestureResponderEvent } from "react-native";
import { draftKey } from "@/fleet";
import { fleet } from "@/runtime";
import { barTop, promptText, tapEndsSelection } from "@/selection";
import { SelectionBar } from "./SelectionBar";

/**
 * What a selected field tells the bar: its text, what is selected now, whether that is on the
 * clipboard, and how to change or end it. `y`: the field's middle in the window (the bar keeps
 * clear of it).
 */
export interface Selected {
  owner: string;
  selected: string;
  copied: boolean;
  /** The field's top and bottom in the window: the bar sits just above it, or just under it. */
  top: number;
  bottom: number;
  line: () => void;
  all: () => void;
  end: () => void;
  /** Measures the field again (the list scrolled under the bar) and shows the bar next to it. */
  relocate: () => void;
}

/** What the pane holds of its scope: `refresh` after a scroll puts the bar back next to the selection. */
export interface SelectionScopeHandle {
  refresh: () => void;
}

interface Scope {
  active: Selected | null;
  show: (s: Selected) => void;
  /** Hides the bar if `owner`'s selection is the one shown; `null`: whichever is shown. */
  clear: (owner: string | null) => void;
}

const ScopeContext = createContext<Scope | null>(null);
const OwnerContext = createContext<string | null>(null);

/** The scope a selectable field reports to, or null outside one (a sheet): then it shows its own "Copied". */
export const useSelectionScope = () => useContext(ScopeContext);
/** Which item of the list a field belongs to (its block), so the others dim while it is selected. */
export const useSelectionOwner = () => useContext(OwnerContext);

/**
 * The pane's output while text in it is selected: the other blocks dim, and a bar says how much
 * is selected and that it is copied, with Line, All, To prompt (when the pane takes commands) and
 * ✕. The bar sits at the bottom of the output, or at its top when the selection is in the lower half.
 */
export function SelectionScope({
  hostKey,
  paneId,
  canPrompt,
  children,
  ref,
}: {
  hostKey: string;
  paneId: number;
  canPrompt: boolean;
  children: ReactNode;
  ref?: Ref<SelectionScopeHandle>;
}) {
  const [active, setActive] = useState<Selected | null>(null);
  const area = useRef<View>(null);
  const [frame, setFrame] = useState({ y: 0, height: 0 });
  const show = useCallback((s: Selected) => setActive(s), []);
  const clear = useCallback((owner: string | null) => setActive((a) => (owner === null || a?.owner === owner ? null : a)), []);
  const scope = useMemo(() => ({ active, show, clear }), [active, show, clear]);
  useImperativeHandle(ref, () => ({ refresh: () => active?.relocate() }), [active]);
  const measure = () => area.current?.measureInWindow((_x, y, _w, height) => setFrame({ y, height }));
  const toPrompt = () => {
    if (!active) return;
    const text = promptText(active.selected);
    if (text === "") return;
    const draft = fleet.store.peek().drafts[draftKey(hostKey, paneId)] ?? "";
    fleet.setDraft(hostKey, paneId, draft === "" || /\s$/.test(draft) ? draft + text : `${draft} ${text}`);
    AccessibilityInfo.announceForAccessibility("Added to the command line");
    active.end();
  };
  // just above the selected field, in thumb reach next to it; under it when there is no room above,
  // and at the bottom when the field runs past both edges
  const [barH, setBarH] = useState(100);
  const place = active ? barTop(active.top - frame.y, active.bottom - frame.y, frame.height, barH, Platform.OS === "android") : undefined;
  // a tap elsewhere in the output (another block's text, the gap between blocks) ends the selection:
  // iOS hides the field's highlight and tells it nothing. A drag (a scroll) keeps it, as iOS does.
  const touch = useRef<{ x: number; y: number } | null>(null);
  const onTouchStart = (e: GestureResponderEvent) => {
    const t = e.nativeEvent.touches;
    touch.current = t.length === 1 ? { x: t[0]!.pageX, y: t[0]!.pageY } : null;
  };
  const onTouchEnd = (e: GestureResponderEvent) => {
    const start = touch.current;
    touch.current = null;
    if (!active || !start || place === undefined) return;
    const { pageX, pageY } = e.nativeEvent;
    if (Math.hypot(pageX - start.x, pageY - start.y) > 10) return;
    // end it, not only the bar: the field still holds the range, and a hold on the same word would change nothing
    if (tapEndsSelection(pageY, active.top, active.bottom, frame.y + place, frame.y + place + barH)) active.end();
  };
  return (
    <ScopeContext.Provider value={scope}>
      <View ref={area} style={styles.fill} onLayout={measure} collapsable={false} onTouchStart={onTouchStart} onTouchEnd={onTouchEnd}>
        {children}
        {active ? (
          <SelectionBar
            selected={active.selected}
            copied={active.copied}
            low
            top={place}
            onHeight={setBarH}
            keys={[
              { label: "Line", hint: "Select the whole line", onPress: active.line },
              { label: "All", hint: "Select all of this text", onPress: active.all },
              ...(canPrompt ? [{ label: "To prompt", hint: "Put the selection in the command line", onPress: toPrompt, sep: true }] : []),
            ]}
            onCancel={active.end}
          />
        ) : null}
      </View>
    </ScopeContext.Provider>
  );
}

/** One item of the list: its fields report as `owner`, and it dims while another item's text is selected. */
export function SelectionItem({ owner, children }: { owner: string; children: ReactNode }) {
  const scope = useSelectionScope();
  const dim = !!scope?.active && scope.active.owner !== owner;
  return (
    <OwnerContext.Provider value={owner}>
      <View style={dim ? styles.dim : null}>{children}</View>
    </OwnerContext.Provider>
  );
}

const styles = StyleSheet.create({
  fill: { flex: 1 },
  dim: { opacity: 0.45 },
});
