import { Text, View } from "react-native";
import { programLine, PROGRAMS, type Program } from "@/launch";
import type { OpenKind } from "@/protocol/types";
import { Tile } from "./Avatar";
import { Button } from "./Button";
import { ErrorText } from "./Notice";
import { CardBox, NavRow } from "./Rows";
import { SheetButtons } from "./SheetFrame";
import { mono, radius, ui, makeStyles, useColors } from "./theme";

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
          leading={<Tile agent={p.id === "claude"} program={p.command} size={28} radius={8} logoSize={16} plain />}
          onPress={() => onPick(p)}
        />
      ))}
    </CardBox>
  );
}

/**
 * Ask before running (Settings): the program's name is shown before it is typed, as the Confirm
 * sheet shows a command. Its Cancel / Run are `ConfirmProgramButtons`, in the sheet's foot.
 */
export function ConfirmProgram({ program, host }: { program: Program; host: string }) {
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
    </View>
  );
}

/** ConfirmProgram's Cancel / Run, for the sheet's foot. Run goes on to Face ID if that is on too. */
export function ConfirmProgramButtons({ busy, onRun, onCancel }: { busy: boolean; onRun: () => void; onCancel: () => void }) {
  const styles = useStyles();
  return (
    <SheetButtons>
      <Button label="Cancel" disabled={busy} onPress={onCancel} style={styles.flex} />
      <Button label="Run" variant="primary" disabled={busy} onPress={onRun} style={styles.flex} />
    </SheetButtons>
  );
}

/** What fills a sheet that cannot start anything: offline, view only, the pane gone. */
export function SheetNote({ children, error }: { children: string; error?: boolean }) {
  const styles = useStyles();
  return error ? <ErrorText>{children}</ErrorText> : <Text style={styles.foot}>{children}</Text>;
}

const useStyles = makeStyles((colors) => ({
  flex: { flex: 1 },
  foot: { fontFamily: ui(400), color: colors.textMuted, fontSize: 13, lineHeight: 18, paddingHorizontal: 4 },
  confirm: { gap: 12 },
  confirmTitle: { fontFamily: ui(600), color: colors.textBright, fontSize: 17 },
  command: { borderRadius: radius.field, backgroundColor: colors.bg, borderWidth: 1, borderColor: colors.hairline, padding: 14 },
  commandText: { fontFamily: mono, color: colors.textBright, fontSize: 14 },
}));
