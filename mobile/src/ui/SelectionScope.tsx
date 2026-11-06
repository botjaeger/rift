import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode, useImperativeHandle, type Ref } from "react";
import { AccessibilityInfo, StyleSheet, View, type GestureResponderEvent } from "react-native";
import { draftKey } from "@/fleet";
import { fleet } from "@/runtime";
import { promptText, tapEndsSelection } from "@/selection";
import { SelectionBar } from "./SelectionBar";

/**
 * What a selected field tells the bar: its text, what is selected now, whether that is on the
 * clipboard, and how to change or end it.
 */
export interface Selected {
  owner: string;
  selected: string;
  copied: boolean;
  /** The field's top and bottom in the window: a tap there (or on its handles) keeps the selection. */
  top: number;
  bottom: number;
  word: () => void;
  line: () => void;
  all: () => void;
  end: () => void;
  /** Measures the field again (the list scrolled it) and reports again. */
  relocate: () => void;
}

/** What the pane holds of its scope: `refresh` after a scroll measures the selected field again. */
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
 * The pane's output while text in it is selected: the other blocks dim, and the selection bar says
 * how much is selected and that it is copied, with Word, Line, All, To prompt (disabled when the
 * pane takes no commands) and ✕. The bar is docked under the output, in the place of the pane's controls
 * (`onSelecting` tells the pane to put them away), so it never covers the text.
 */
export function SelectionScope({
  hostKey,
  paneId,
  canPrompt,
  onBar,
  children,
  ref,
}: {
  hostKey: string;
  paneId: number;
  canPrompt: boolean;
  /**
   * The selection bar while text is selected, else null: the pane draws it over its controls
   * (hidden, still in place), so nothing in the output moves when a selection starts.
   */
  onBar: (bar: ReactNode | null) => void;
  children: ReactNode;
  ref?: Ref<SelectionScopeHandle>;
}) {
  const [active, setActive] = useState<Selected | null>(null);
  const shown = useRef<Selected | null>(null);
  useEffect(() => {
    shown.current = active;
  }, [active]);
  // A selection in another field ends the one before it properly: iOS hides the old field's
  // highlight and tells it nothing, so it kept its range, and a hold on the same word there later
  // set that range again: no change, no event, a selection with no bar.
  const show = useCallback((s: Selected) => {
    const prev = shown.current;
    if (prev && prev.owner !== s.owner) prev.end();
    shown.current = s;
    setActive(s);
  }, []);
  const clear = useCallback((owner: string | null) => setActive((a) => (owner === null || a?.owner === owner ? null : a)), []);
  const scope = useMemo(() => ({ active, show, clear }), [active, show, clear]);
  useImperativeHandle(ref, () => ({ refresh: () => active?.relocate() }), [active]);
  useEffect(() => {
    if (!active) return onBar(null);
    const toPrompt = () => {
      const text = promptText(active.selected);
      if (text === "") return;
      const draft = fleet.store.peek().drafts[draftKey(hostKey, paneId)] ?? "";
      fleet.setDraft(hostKey, paneId, draft === "" || /\s$/.test(draft) ? draft + text : `${draft} ${text}`);
      AccessibilityInfo.announceForAccessibility("Added to the command line");
      active.end();
    };
    onBar(
      <SelectionBar
        selected={active.selected}
        copied={active.copied}
        of="block"
        onWord={active.word}
        onLine={active.line}
        onAll={active.all}
        onPrompt={canPrompt ? toPrompt : undefined}
        promptOff="Nothing can be typed in this pane."
        onCancel={active.end}
      />,
    );
  }, [active, canPrompt, hostKey, paneId, onBar]);
  // the blocks give way to a program's screen mid-selection: the controls come back
  useEffect(() => () => onBar(null), [onBar]);
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
    if (!active || !start) return;
    const { pageX, pageY } = e.nativeEvent;
    if (Math.hypot(pageX - start.x, pageY - start.y) > 10) return;
    // end it, not only the bar: the field still holds the range, and a hold on the same word would
    // change nothing. The bar is outside the output: no touch on it reaches here.
    if (tapEndsSelection(pageY, active.top, active.bottom, -1, -1)) active.end();
  };
  return (
    <ScopeContext.Provider value={scope}>
      <View style={styles.fill} onTouchStart={onTouchStart} onTouchEnd={onTouchEnd}>
        {children}
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
