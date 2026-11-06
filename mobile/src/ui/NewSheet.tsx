import type { ReactNode } from "react";
import { ScrollView, Text, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { programLine, PROGRAMS, type Program } from "@/launch";
import type { OpenKind } from "@/protocol/types";
import { useLock } from "@/runtime";
import { Tile } from "./Avatar";
import { Button } from "./Button";
import { ErrorText } from "./Notice";
import { CardBox, NavRow } from "./Rows";
import { SheetHeader } from "./SheetHeader";
import { mono, radius, ui, makeStyles, useColors } from "./theme";

/**
 * The frame of the New tab, New pane and Attach sheets: the sheet header (grabber, title,
 * subtitle, ✕), then the content. The sheet's own ScrollView is its root: iOS lays out a form
 * sheet's first scroll view as the sheet's content.
 */
export function SheetFrame({
  title,
  subtitle,
  onClose,
  children,
}: {
  title: string;
  subtitle: string;
  onClose: () => void;
  children: ReactNode;
}) {
  const styles = useStyles();
  const insets = useSafeAreaInsets();
  const lock = useLock();
  const hidden = lock.covered || lock.locked;
  return (
    <ScrollView
      style={[styles.fill, hidden && styles.hidden]}
      contentContainerStyle={{ paddingBottom: Math.max(insets.bottom, 30) }}
      accessibilityElementsHidden={hidden}
      importantForAccessibility={hidden ? "no-hide-descendants" : "auto"}
    >
      <SheetHeader title={title} subtitle={subtitle} onClose={onClose} />
      <View style={styles.body}>{children}</View>
    </ScrollView>
  );
}

export function Footnote({ children }: { children: string }) {
  const styles = useStyles();
  return <Text style={styles.foot}>{children}</Text>;
}

/** The four starts, as one card of rows: the program's mark, its name, what it does. */
export function ProgramRows({ kind, disabled, onPick }: { kind: OpenKind; disabled: boolean; onPick: (p: Program) => void }) {
  return (
    <CardBox>
      {PROGRAMS.map((p, i) => (
        <NavRow
          key={p.id}
          first={i === 0}
          label={p.title}
          detail={programLine(p, kind)}
          accessibilityLabel={`${p.title}. ${programLine(p, kind)}`}
          disabled={disabled}
          leading={<Tile agent={p.id === "claude"} program={p.command} size={32} radius={8} promptSize={12} />}
          onPress={() => onPick(p)}
        />
      ))}
    </CardBox>
  );
}

/**
 * Ask before running (Settings): the program's name is shown before it is typed, as the Confirm
 * sheet shows a command. Run goes on to Face ID if that is on too.
 */
export function ConfirmProgram({ program, host, busy, onRun, onCancel }: { program: Program; host: string; busy: boolean; onRun: () => void; onCancel: () => void }) {
  const colors = useColors();
  const styles = useStyles();
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
      <View style={styles.buttons}>
        <Button label="Cancel" disabled={busy} onPress={onCancel} style={styles.flex} />
        <Button label="Run" variant="primary" disabled={busy} onPress={onRun} style={styles.flex} />
      </View>
    </View>
  );
}

/** What fills a sheet that cannot start anything: offline, view only, the pane gone. */
export function SheetNote({ children, error }: { children: string; error?: boolean }) {
  const styles = useStyles();
  return error ? <ErrorText>{children}</ErrorText> : <Text style={styles.foot}>{children}</Text>;
}

const useStyles = makeStyles((colors) => ({
  fill: { flex: 1, backgroundColor: colors.surface },
  hidden: { opacity: 0 },
  flex: { flex: 1 },
  body: { gap: 12, paddingHorizontal: 16, maxWidth: 560, width: "100%", alignSelf: "center" },
  foot: { fontFamily: ui(400), color: colors.textMuted, fontSize: 13, lineHeight: 18, paddingHorizontal: 4 },
  confirm: { gap: 12 },
  confirmTitle: { fontFamily: ui(600), color: colors.textBright, fontSize: 17 },
  command: { borderRadius: radius.field, backgroundColor: colors.bg, borderWidth: 1, borderColor: colors.hairline, padding: 14 },
  commandText: { fontFamily: mono, color: colors.textBright, fontSize: 14 },
  buttons: { flexDirection: "row", gap: 8 },
}));
