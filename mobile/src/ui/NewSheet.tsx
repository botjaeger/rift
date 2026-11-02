import type { ReactNode } from "react";
import { Pressable, ScrollView, StyleSheet, Text, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { programLine, PROGRAMS, type Program } from "@/launch";
import type { OpenKind } from "@/protocol/types";
import { useLock } from "@/runtime";
import { Tile } from "./Avatar";
import { Button } from "./Button";
import { Icon } from "./Icon";
import { ErrorText } from "./Notice";
import { colors, mono, TOUCH, ui } from "./theme";

/**
 * The frame of the New tab, New pane and Attach sheets (the boards): the title with the
 * computer's dot and a line under it, the 44 pt Close, then the content. The sheet's own
 * ScrollView is its root: iOS lays out a form sheet's first scroll view as the sheet's content.
 */
export function SheetFrame({
  title,
  subtitle,
  online,
  onClose,
  children,
}: {
  title: string;
  subtitle: string;
  online: boolean;
  onClose: () => void;
  children: ReactNode;
}) {
  const insets = useSafeAreaInsets();
  const lock = useLock();
  const hidden = lock.covered || lock.locked;
  return (
    <ScrollView
      style={[styles.fill, hidden && styles.hidden]}
      contentContainerStyle={[styles.sheet, { paddingBottom: Math.max(insets.bottom, 30) }]}
      accessibilityElementsHidden={hidden}
      importantForAccessibility={hidden ? "no-hide-descendants" : "auto"}
    >
      <View style={styles.header}>
        <View style={styles.titles}>
          <Text style={styles.title} accessibilityRole="header">
            {title}
          </Text>
          <View style={styles.subtitle}>
            <View style={[styles.dot, { backgroundColor: online ? colors.success : colors.textDim }]} />
            <Text style={styles.subtitleText} numberOfLines={1}>
              {subtitle}
            </Text>
          </View>
        </View>
        <Pressable accessibilityRole="button" accessibilityLabel="Close" onPress={onClose} style={({ pressed }) => [styles.closeSlot, pressed && styles.pressed]}>
          <View style={styles.close}>
            <Icon name="close" size={16} color={colors.text} />
          </View>
        </Pressable>
      </View>
      <View style={styles.body}>{children}</View>
    </ScrollView>
  );
}

/** The grouped card of rows the boards draw (hairline border, rows ruled apart). */
export function Group({ children }: { children: ReactNode }) {
  return <View style={styles.group}>{children}</View>;
}

/** A label and its value on one line (Opens on, Starts in). */
export function InfoRow({ label, value, first, path }: { label: string; value: string; first?: boolean; path?: boolean }) {
  return (
    <View style={[styles.info, !first && styles.rule]} accessible accessibilityLabel={`${label}: ${value}`}>
      <Text style={styles.infoLabel}>{label}</Text>
      {/* a long path keeps its last folders on one line, so the rows stay on the sheet; the label reads it whole */}
      <Text style={styles.infoValue} numberOfLines={path ? 1 : undefined} ellipsizeMode={path ? "head" : undefined}>
        {value}
      </Text>
    </View>
  );
}

export const SectionLabel = ({ children }: { children: string }) => <Text style={styles.label}>{children}</Text>;
export const Footnote = ({ children }: { children: string }) => <Text style={styles.foot}>{children}</Text>;

/** The four starts, as rows: the program's mark, its name, what it does. */
export function ProgramRows({ kind, disabled, onPick }: { kind: OpenKind; disabled: boolean; onPick: (p: Program) => void }) {
  return (
    <Group>
      {PROGRAMS.map((p, i) => (
        <Pressable
          key={p.id}
          accessibilityRole="button"
          accessibilityLabel={`${p.title}. ${programLine(p, kind)}`}
          accessibilityState={{ disabled }}
          disabled={disabled}
          onPress={() => onPick(p)}
          style={({ pressed }) => [styles.program, i > 0 && styles.rule, disabled && styles.disabled, pressed && styles.pressed]}
        >
          <Tile agent={p.id === "claude"} program={p.command} size={36} radius={10} promptSize={13} />
          <View style={styles.programText}>
            <Text style={styles.programTitle}>{p.title}</Text>
            <Text style={styles.programLine}>{programLine(p, kind)}</Text>
          </View>
          <Icon name="forward" size={14} color={colors.textMuted} />
        </Pressable>
      ))}
    </Group>
  );
}

/**
 * Ask before running (Settings): the program's name is shown before it is typed, as the Confirm
 * sheet shows a command. Run goes on to Face ID if that is on too.
 */
export function ConfirmProgram({ program, host, busy, onRun, onCancel }: { program: Program; host: string; busy: boolean; onRun: () => void; onCancel: () => void }) {
  return (
    <View style={styles.confirm}>
      <Text style={styles.confirmTitle} accessibilityRole="header">{`Run ${program.command} on ${host}?`}</Text>
      <View style={styles.command} accessible accessibilityLabel={`Command: ${program.command}`}>
        <Text style={styles.commandText}>
          <Text style={{ color: colors.success }}>❯ </Text>
          {program.command}
        </Text>
      </View>
      <Text style={styles.foot}>Shown because Settings › Ask before running is on. It is typed in the new shell.</Text>
      <Button label="Run" variant="primary" disabled={busy} onPress={onRun} />
      <Button label="Cancel" disabled={busy} onPress={onCancel} />
    </View>
  );
}

/** What fills a sheet that cannot start anything: offline, view only, the pane gone. */
export function SheetNote({ children, error }: { children: string; error?: boolean }) {
  return error ? <ErrorText>{children}</ErrorText> : <Text style={styles.foot}>{children}</Text>;
}

const styles = StyleSheet.create({
  pressed: { opacity: 0.75 },
  disabled: { opacity: 0.45 },
  fill: { flex: 1, backgroundColor: colors.surface },
  hidden: { opacity: 0 },
  sheet: { paddingTop: 20, paddingHorizontal: 16, gap: 12 },
  header: { flexDirection: "row", alignItems: "center", gap: 10 },
  titles: { flex: 1, minWidth: 0, gap: 3 },
  title: { fontFamily: ui(600), color: colors.textBright, fontSize: 20 },
  subtitle: { flexDirection: "row", alignItems: "center", gap: 6 },
  dot: { width: 7, height: 7, borderRadius: 4 },
  subtitleText: { flexShrink: 1, fontFamily: ui(400), color: colors.textMuted, fontSize: 13 },
  // the board's 40 pt badge in its 44 pt slot
  closeSlot: { minWidth: TOUCH, minHeight: TOUCH, alignItems: "center", justifyContent: "center" },
  close: { width: 40, height: 40, borderRadius: 20, backgroundColor: colors.glass, borderWidth: 1, borderColor: colors.glassRim, alignItems: "center", justifyContent: "center" },
  body: { gap: 10, paddingTop: 2 },
  group: { borderRadius: 12, backgroundColor: colors.bg, borderWidth: 1, borderColor: colors.hairline, overflow: "hidden" },
  rule: { borderTopWidth: 1, borderTopColor: colors.hairline },
  info: { flexDirection: "row", alignItems: "center", gap: 12, minHeight: TOUCH, paddingVertical: 8, paddingHorizontal: 14 },
  infoLabel: { minWidth: 84, fontFamily: ui(400), color: colors.textMuted, fontSize: 14 },
  infoValue: { flex: 1, textAlign: "right", fontFamily: ui(400), color: colors.textBright, fontSize: 14 },
  label: { fontFamily: ui(500), color: colors.textMuted, fontSize: 13, paddingTop: 6, paddingHorizontal: 4 },
  foot: { fontFamily: ui(400), color: colors.textMuted, fontSize: 12, lineHeight: 17, paddingHorizontal: 4 },
  program: { flexDirection: "row", alignItems: "center", gap: 12, minHeight: 60, paddingVertical: 8, paddingHorizontal: 14 },
  programText: { flex: 1, minWidth: 0, gap: 2 },
  programTitle: { fontFamily: ui(600), color: colors.textBright, fontSize: 15 },
  programLine: { fontFamily: ui(400), color: colors.textMuted, fontSize: 12, lineHeight: 16 },
  confirm: { gap: 12 },
  confirmTitle: { fontFamily: ui(600), color: colors.textBright, fontSize: 17 },
  command: { borderRadius: 12, backgroundColor: colors.bg, borderWidth: 1, borderColor: colors.hairline, padding: 14 },
  commandText: { fontFamily: mono, color: colors.textBright, fontSize: 14 },
});
