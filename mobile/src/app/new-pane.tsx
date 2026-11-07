import { useLocalSearchParams } from "expo-router";
import { tildePath } from "@/chips";
import { canOperate } from "@/fleet";
import { useApp } from "@/runtime";
import { findPane, tabTitle } from "@/topology";
import { Button } from "@/ui/Button";
import { ErrorText } from "@/ui/Notice";
import { canNewPane } from "@/ui/newTab";
import { ConfirmProgram, ConfirmProgramButtons, Footnote, ProgramRows, SheetNote } from "@/ui/NewSheet";
import { SheetFrame } from "@/ui/SheetFrame";
import { useLaunch } from "@/ui/useLaunch";

/**
 * The New pane sheet (the New pane board), opened from the Panes sheet: what the new pane
 * starts with, a shell or a coding agent typed into it. The pane opens beside the one the phone
 * is on, in its folder (the subtitle), and the phone goes to it.
 */
export default function NewPaneSheet() {
  const { key, pane: paneParam } = useLocalSearchParams<{ key: string; pane: string }>();
  const paneId = Number(paneParam);
  const s = useApp();
  const host = s.hosts.find((h) => h.record.key === key);
  const loc = host ? findPane(host.windows, paneId) : null;
  const launch = useLaunch({ host, kind: "pane", pane: Number.isSafeInteger(paneId) ? paneId : null, from: undefined });
  const online = host?.conn.status === "open";
  const name = host?.record.name ?? "";
  const cwd = loc?.pane.cwd ? tildePath(loc.pane.cwd) : null;

  let body;
  let footer = null;
  if (!host) body = <SheetNote error>Computer not found.</SheetNote>;
  else if (!loc) body = <SheetNote>This pane has closed.</SheetNote>;
  else if (!online) body = <SheetNote>{`${name} is offline. Open this again when it is back.`}</SheetNote>;
  else if (!canOperate(host)) body = <SheetNote>This device can only view; it cannot open panes.</SheetNote>;
  else if (!canNewPane(host, paneId))
    body = <SheetNote>{`Update Rift on ${name} to open panes from the phone.`}</SheetNote>;
  else if (launch.ask) {
    body = <ConfirmProgram program={launch.ask} host={name} />;
    footer = <ConfirmProgramButtons busy={launch.busy} onRun={() => void launch.go(launch.ask!)} onCancel={() => launch.setAsk(null)} />;
  } else
    body = (
      <>
        {/* the same rows as New tab; where it starts is in the subtitle */}
        <ProgramRows kind="pane" disabled={launch.busy} onPick={launch.pick} />
        {launch.busy ? <Footnote>Opening…</Footnote> : null}
      </>
    );
  if (launch.opened !== null) footer = <Button label="Go to the new pane" onPress={() => launch.finish(launch.opened ?? undefined)} />;

  return (
    <SheetFrame
      board="newPane"
      title="New pane"
      subtitle={loc ? `Beside ${tabTitle(loc.tab)} · ${cwd ?? name}${online ? "" : " · offline"}` : name || "Computer not found"}
      onClose={launch.close}
      footer={footer}
    >
      {body}
      {launch.error ? <ErrorText>{launch.error}</ErrorText> : null}
    </SheetFrame>
  );
}
