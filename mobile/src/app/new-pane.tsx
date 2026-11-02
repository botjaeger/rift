import { useLocalSearchParams } from "expo-router";
import { tildePath } from "@/chips";
import { canOperate } from "@/fleet";
import { useApp } from "@/runtime";
import { findPane, tabTitle } from "@/topology";
import { Button } from "@/ui/Button";
import { ErrorText } from "@/ui/Notice";
import { canNewPane } from "@/ui/newTab";
import { ConfirmProgram, Footnote, ProgramRows, SectionLabel, SheetFrame, SheetNote } from "@/ui/NewSheet";
import { useLaunch } from "@/ui/useLaunch";

/**
 * The New pane sheet (the New pane board), opened from the Panes sheet: what the new pane
 * starts with, a shell or a coding agent typed into it. The pane opens beside the one the phone
 * is on, in its folder, and the phone goes to it.
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
  if (!host) body = <SheetNote error>Computer not found.</SheetNote>;
  else if (!loc) body = <SheetNote>This pane has closed.</SheetNote>;
  else if (!online) body = <SheetNote>{`${name} is offline. Open this again when it is back.`}</SheetNote>;
  else if (!canOperate(host)) body = <SheetNote>This device can only view; it cannot open panes.</SheetNote>;
  else if (!canNewPane(host, paneId))
    body = <SheetNote>{`Update Rift on ${name} to open panes from the phone.`}</SheetNote>;
  else if (launch.ask)
    body = (
      <ConfirmProgram program={launch.ask} host={name} busy={launch.busy} onRun={() => void launch.go(launch.ask!)} onCancel={() => launch.setAsk(null)} />
    );
  else
    body = (
      <>
        <SectionLabel>Start with</SectionLabel>
        <ProgramRows kind="pane" disabled={launch.busy} onPick={launch.pick} />
        {launch.busy ? <Footnote>Opening…</Footnote> : null}
        <Footnote>{`The new pane starts in ${cwd ?? "the same folder"}, and this phone goes to it.`}</Footnote>
      </>
    );

  return (
    <SheetFrame title="New pane" subtitle={loc ? `${tabTitle(loc.tab)} · ${name}` : name || "Computer not found"} online={!!online} onClose={launch.close}>
      {body}
      {launch.error ? <ErrorText>{launch.error}</ErrorText> : null}
      {launch.opened !== null ? <Button label="Go to the new pane" onPress={() => launch.finish(launch.opened ?? undefined)} /> : null}
    </SheetFrame>
  );
}
