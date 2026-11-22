import * as Clipboard from "expo-clipboard";
import { useEffect, useId, useMemo, useRef, useState } from "react";
import { AccessibilityInfo, Linking, Platform, Text, TextInput, View, type TextInputSelectionChangeEvent, type TextStyle } from "react-native";
import * as haptics from "@/haptics";
import { textParts } from "@/paneView";
import { linkAtOffset, openableLink, textLinks } from "@/terminal";
import { COPY_SETTLE_MS, lineRange, selectedText, showsOwnCopied, textToCopy, wordRange } from "@/selection";
import { partChildren, useMarkStyle } from "./Marked";
import { useSelectionOwner, useSelectionScope } from "./SelectionScope";
import { COPIED_MS } from "./useFlash";
import { mono, ui, makeStyles, useColors } from "./theme";


type Range = { start: number; end: number };
// how long a caret in a link waits for the selection a hold brings (13 ms on API 35) before it opens
const LINK_TAP_MS = 150;
// the field's handle (named: test/fonts.test.ts reads every TextInput tag as an element)
type Field = TextInput;

/**
 * Block output you can select, and selecting copies it: a read-only field (`Text selectable`
 * only offers the system's Copy in a menu), whose selection is copied `COPY_SETTLE_MS` after the
 * handles stop, with a small vibration when it lands. A tap puts a caret, which is an empty
 * selection and copies nothing. The text read is the field's own at that moment, so output that
 * changed meanwhile copies what is shown. `lines` limits the height (a running card's single line).
 *
 * Inside a SelectionScope (the pane's output) the scope's bar says how much is selected and
 * offers Word, Line, All, To prompt and ✕, and the system's edit menu stays hidden (it opened over the
 * bar and offered Copy again); elsewhere (a sheet) the field shows its own "Copied" and the menu.
 *
 * `mark` (the Search output query) marks each match, and http(s) links are drawn in the accent,
 * underlined: the field then holds the text as styled pieces instead of `value`, the same
 * characters, so selection offsets and copying are unchanged. A tap on a link (its caret lands in
 * it) opens it in the browser; a hold still selects. A screen reader opens the first link with
 * the field's "Open link" action.
 */
export function SelectableText({
  text,
  style,
  lineHeight,
  lines,
  mark,
}: {
  text: string;
  style: TextStyle;
  lineHeight: number;
  lines?: number;
  mark?: string;
}) {
  const colors = useColors();
  const styles = useStyles();
  const markStyle = useMarkStyle();
  const links = useMemo(() => textLinks(text), [text]);
  const parts = useMemo(() => textParts(text, mark, links), [text, mark, links]);
  const shown = useRef(text);
  useEffect(() => {
    shown.current = text;
  }, [text]);
  const scope = useSelectionScope();
  const ownId = useId();
  const owner = useSelectionOwner() ?? ownId;
  const field = useRef<Field>(null);
  const wrap = useRef<View>(null);
  const copied = useRef<string | null>(null);
  const range = useRef<Range>({ start: 0, end: 0 });
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const hide = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const [notice, setNotice] = useState(false);
  // a selection the bar set (Line, All); the field follows the finger otherwise
  const [forced, setForced] = useState<Range | undefined>(undefined);
  // the whole text's height, for a field `lines` cuts (below)
  const [full, setFull] = useState<number | null>(null);
  // the latest scope and owner for the timers and the bar's callbacks
  const live = useRef({ scope, owner });
  useEffect(() => {
    live.current = { scope, owner };
  });
  useEffect(
    () => () => {
      clearTimeout(timer.current);
      clearTimeout(hide.current);
      live.current.scope?.clear(live.current.owner);
    },
    [],
  );
  // what the bar last said: a relocation after a scroll keeps it
  const lastCopied = useRef(false);
  const report = (sel: Range, isCopied: boolean) => {
    const { scope: sc, owner: own } = live.current;
    if (!sc) return;
    lastCopied.current = isCopied;
    wrap.current?.measureInWindow((_x, y, _w, h) => {
      sc.show({
        owner: own,
        selected: selectedText(shown.current, sel),
        copied: isCopied,
        top: y,
        bottom: y + h,
        word: () => select(wordRange(shown.current, range.current)),
        line: () => select(lineRange(shown.current, range.current)),
        all: () => select({ start: 0, end: shown.current.length }),
        end,
        relocate: () => report(range.current, lastCopied.current),
      });
    });
  };
  const copy = (sel: Range) => {
    const t = textToCopy(selectedText(shown.current, sel), copied.current);
    if (t === null) {
      report(sel, copied.current !== null && copied.current === selectedText(shown.current, sel));
      return;
    }
    copied.current = t;
    void Clipboard.setStringAsync(t).then(
      () => {
        haptics.copied();
        AccessibilityInfo.announceForAccessibility("Copied");
        report(sel, true);
        if (live.current.scope || !showsOwnCopied(Platform.OS, Platform.Version)) return;
        setNotice(true);
        clearTimeout(hide.current);
        hide.current = setTimeout(() => setNotice(false), COPIED_MS);
      },
      () => {
        AccessibilityInfo.announceForAccessibility("Could not copy");
        report(sel, false);
      },
    );
  };
  const select = (sel: Range) => {
    clearTimeout(timer.current);
    range.current = sel;
    setForced(sel);
    copy(sel);
  };
  const end = () => {
    clearTimeout(timer.current);
    copied.current = null;
    range.current = { start: 0, end: 0 };
    setForced({ start: 0, end: 0 });
    field.current?.blur();
    live.current.scope?.clear(live.current.owner);
  };
  const onSelection = (e: TextInputSelectionChangeEvent) => {
    const sel = e.nativeEvent.selection;
    // a caret after a selection (Android's Back ending it) is no tap: it never opens a link, and
    // neither does one the field was given unfocused (Android puts one at the end when it mounts)
    const wasSelection = range.current.start !== range.current.end;
    const touched = field.current?.isFocused() ?? false;
    range.current = sel;
    clearTimeout(timer.current);
    if (forced && (forced.start !== sel.start || forced.end !== sel.end)) setForced(undefined);
    if (sel.start === sel.end) {
      copied.current = null;
      live.current.scope?.clear(live.current.owner);
      // Android: Back ends the edit menu's selection mode but the field stays focused, and drew a
      // caret in the output (caretHidden or not) until something else took the focus
      if (Platform.OS === "android") field.current?.blur();
      // a tap on a link (Android: its caret stands in it; iOS opens links itself, below) opens it,
      // once no selection follows: a hold puts the caret there first, then selects the word
      const url = wasSelection || !touched ? null : linkAtOffset(links, sel.start);
      if (url && openableLink(url)) {
        timer.current = setTimeout(() => {
          field.current?.blur();
          void Linking.openURL(url);
        }, LINK_TAP_MS);
      }
      return;
    }
    timer.current = setTimeout(() => copy(sel), COPY_SETTLE_MS);
  };
  return (
    <View ref={wrap} collapsable={false}>
      {/* `lines` clips the field from outside, the field keeps its whole text's height: one held
          shorter (Android measures it against the space it gets) scrolls its text to the caret, so
          a cut line showed its end and a tap moved it */}
      <View style={lines ? { maxHeight: lineHeight * lines, overflow: "hidden" } : null}>
        <TextInput
          keyboardAppearance={colors.dark ? "dark" : "light"}
          ref={field}
          // a TextInput takes `value` or styled children, never both
          value={parts ? undefined : text}
          multiline
          // React Native's read-only on Android is a disabled view, which cannot select: there the field
          // stays enabled with no keyboard, and `value` undoes anything a hardware keyboard types
          readOnly={Platform.OS !== "android"}
          // iOS: a read-only field takes no caret from a tap, so the system's link detection makes the
          // links tappable there (Android's enabled field puts a caret in a tapped link: onSelection)
          dataDetectorTypes={Platform.OS === "ios" && links.length > 0 ? "link" : undefined}
          caretHidden={Platform.OS === "android"}
          scrollEnabled={false}
          showSoftInputOnFocus={false}
          selection={forced}
          // iOS draws detected links in the field's tint, which is this colour: a field with links takes
          // the accent (its selection is then the accent, lightened by the system), the rest the selection colour
          selectionColor={Platform.OS === "ios" && links.length > 0 ? colors.accent : colors.selection}
          // Android: the handles in the accent (the selection board's knobs), apart from the ground;
          // iOS draws its own in the selection colour (no prop sets them apart there)
          selectionHandleColor={colors.accent}
          // iOS only: Android's selection lives in the edit menu's mode, and hiding the menu ends it at once
          contextMenuHidden={scope !== null && Platform.OS === "ios"}
          onSelectionChange={onSelection}
          onContentSizeChange={lines ? (e) => setFull(Math.ceil(e.nativeEvent.contentSize.height)) : undefined}
          style={[styles.field, style, { lineHeight }, lines && full ? { height: full } : null]}
          accessibilityActions={links.length > 0 ? [{ name: "openLink", label: `Open ${links[0]!.url}` }] : undefined}
          onAccessibilityAction={(e) => {
            const url = links[0]?.url;
            if (e.nativeEvent.actionName === "openLink" && url && openableLink(url)) void Linking.openURL(url);
          }}
        >
          {parts ? partChildren(parts, markStyle.mark, styles.link) : null}
        </TextInput>
      </View>
      {notice ? (
        <View pointerEvents="none" style={styles.notice}>
          <Text style={styles.noticeText}>Copied</Text>
        </View>
      ) : null}
    </View>
  );
}

const useStyles = makeStyles((colors) => ({
  // a field draws its own padding: a block's text sits flush like Text
  // the caller's style names the family; mono is the fallback
  field: { padding: 0, margin: 0, textAlignVertical: "top", fontFamily: mono },
  notice: { position: "absolute", top: 0, right: 0, paddingHorizontal: 8, paddingVertical: 2, borderRadius: 10, backgroundColor: colors.surface, borderWidth: 1, borderColor: colors.glassRim },
  noticeText: { fontFamily: ui(500), color: colors.textBright, fontSize: 12 },
  // a link in the output: the accent, underlined (as on a program's screen)
  link: { color: colors.accent, textDecorationLine: "underline" },
}));
