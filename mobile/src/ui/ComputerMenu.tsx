import { Pressable, Text, View } from "react-native";
import type { HostView } from "@/fleet";
import { tabPane } from "@/now";
import { hostLine, lastSeenAt } from "./connStatus";
import { hostDot } from "./hostDot";
import { glyph } from "./Screen";
import { Icon } from "./Icon";
import { ui, makeStyles, useColors } from "./theme";

/** The pane another computer opens on: its active tab's (the first window's), else its first tab's. */
export function landingPane(host: HostView): number | null {
  const tabs = host.windows.flatMap((w) => w.tabs);
  const tab = tabs.find((t) => t.active) ?? tabs[0];
  return tab ? tabPane(tab) : null;
}

/**
 * The computer menu (the pane header's name): every paired computer with how it is reached, the
 * one on screen checked; a connected one opens on its active tab. Then Connection (how this phone
 * reaches the one on screen) and All portals.
 */
export function ComputerMenu({
  hosts,
  current,
  now,
  top,
  onOpen,
  onConnection,
  onPortals,
  onDismiss,
}: {
  hosts: readonly HostView[];
  current: string;
  now: number;
  top: number;
  onOpen: (host: string, pane: number) => void;
  onConnection: () => void;
  onPortals: () => void;
  /** The computer on screen was picked: the menu closes. */
  onDismiss: () => void;
}) {
  const colors = useColors();
  const styles = useStyles();
  const here = hosts.find((h) => h.record.key === current);
  return (
    <View accessibilityRole="menu" accessibilityLabel="Computers" style={[styles.menu, { top }]}>
      <Text style={styles.cap}>Computers</Text>
      {hosts.map((h) => {
        const on = h.record.key === current;
        const tabs = h.windows.reduce((n, w) => n + w.tabs.length, 0);
        const open = h.conn.status === "open";
        const pane = open ? landingPane(h) : null;
        // where it is: "This network · 30 tabs", "Over the relay", "Can’t reach it · last seen 2 h ago"
        const line = hostLine(h.conn, now, open ? tabs : null, lastSeenAt(h)).text;
        const usable = on || pane !== null;
        return (
          <Pressable
            key={h.record.key}
            accessibilityRole="menuitem"
            accessibilityState={{ checked: on }}
            aria-checked={on}
            accessibilityLabel={`${h.record.name}, ${line}`}
            // one it cannot open: Portals, where its card says why and has Try again
            accessibilityHint={usable ? undefined : "Opens Portals"}
            onPress={() => (on ? onDismiss() : pane !== null ? onOpen(h.record.key, pane) : onPortals())}
            style={({ pressed }) => [styles.row, styles.rule, pressed && styles.pressed]}
          >
            <View style={styles.dotBox}>
              <View style={[styles.dot, { backgroundColor: colors[hostDot(h)] }]} />
            </View>
            <View style={styles.text}>
              <Text style={styles.title} numberOfLines={1}>
                {h.record.name}
              </Text>
              <Text style={styles.hint} numberOfLines={1}>
                {line}
              </Text>
            </View>
            {on ? <Icon name="check" size={glyph(16, 11)} color={colors.accent} /> : null}
          </Pressable>
        );
      })}
      {here ? (
        <Pressable accessibilityRole="menuitem" accessibilityLabel={`Connection: how this phone reaches ${here.record.name}`} onPress={onConnection} style={({ pressed }) => [styles.row, styles.rule, pressed && styles.pressed]}>
          <View style={styles.dotBox}>
            <Icon name="wifi" size={glyph(18, 15)} color={colors.text} />
          </View>
          <View style={styles.text}>
            <Text style={styles.title}>Connection</Text>
            {/* the computer is named in the row above: the line says what the sheet holds */}
            <Text style={styles.hint} numberOfLines={1}>
              How it is reached, and how fast
            </Text>
          </View>
        </Pressable>
      ) : null}
      <Pressable accessibilityRole="menuitem" accessibilityLabel="All portals" onPress={onPortals} style={({ pressed }) => [styles.row, pressed && styles.pressed]}>
        <View style={styles.dotBox}>
          <Icon name="computer" size={glyph(18, 15)} color={colors.text} />
        </View>
        <View style={styles.text}>
          <Text style={styles.title}>All portals</Text>
        </View>
      </Pressable>
    </View>
  );
}

const useStyles = makeStyles((colors) => ({
  // the New Portals board's menu, under the header's name: one hairline between every row
  menu: {
    position: "absolute",
    left: 12,
    width: 300,
    borderRadius: 14,
    backgroundColor: colors.menu,
    borderWidth: 1,
    borderColor: colors.borderStrong,
    overflow: "hidden",
    // the board's 0 12 40 #000000aa
    shadowColor: "#000000",
    shadowOpacity: 0.67,
    shadowRadius: 20,
    shadowOffset: { width: 0, height: 12 },
    elevation: 16,
  },
  // the Standard's caption: Inter 500 12/16, uppercase
  cap: { fontFamily: ui(500), color: colors.textMuted, fontSize: 12, lineHeight: 16, textTransform: "uppercase", paddingHorizontal: 16, paddingTop: 12, paddingBottom: 6 },
  // 68 pt: 52 and 8 above and under
  row: { minHeight: 68, flexDirection: "row", alignItems: "center", gap: 12, paddingHorizontal: 16, paddingVertical: 8 },
  rule: { borderBottomWidth: 1, borderBottomColor: colors.hairline },
  pressed: { opacity: 0.7 },
  dotBox: { width: 20, alignItems: "center" },
  dot: { width: 8, height: 8, borderRadius: 4 },
  text: { flex: 1, minWidth: 0, gap: 2 },
  title: { fontFamily: ui(400), color: colors.text, fontSize: 16, lineHeight: 21 },
  hint: { fontFamily: ui(400), color: colors.textMuted, fontSize: 13, lineHeight: 17 },
}));
