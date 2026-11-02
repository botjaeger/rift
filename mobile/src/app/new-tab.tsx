import { useLocalSearchParams } from "expo-router";
import { canOperate } from "@/fleet";
import { tildePath } from "@/chips";
import { useApp } from "@/runtime";
import { Button } from "@/ui/Button";
import { ErrorText } from "@/ui/Notice";
import { basePane, newTabCwd } from "@/ui/newTab";
import { ConfirmProgram, Footnote, Group, InfoRow, ProgramRows, SectionLabel, SheetFrame, SheetNote } from "@/ui/NewSheet";
import { canStartTab, StartTab } from "@/ui/StartTab";
import { useLaunch } from "@/ui/useLaunch";

/**
 * The New tab sheet (the New tab boards), opened from the Tabs sheet or a computer's card on
 * Portals (`from=home`): what the new tab starts with, a shell or a coding agent typed into it.
 * With Rift closed on the computer it is the Open a tab step instead (a shell only: the computer
 * keeps no program to start for a phone while Rift is closed). View-only devices get no start.
 */
export default function NewTabSheet() {
  const { key, from } = useLocalSearchParams<{ key: string; from?: string }>();
  const s = useApp();
  const host = s.hosts.find((h) => h.record.key === key);
  const base = host ? basePane(host) : null;
  const launch = useLaunch({ host, kind: "tab", pane: base, from });
  const online = host?.conn.status === "open";
  const name = host?.record.name ?? "";
  const cwd = host ? newTabCwd(host) : null;
  const closed = !!host && canStartTab(host);

  let body;
  if (!host) body = <SheetNote error>Computer not found.</SheetNote>;
  else if (!online) body = <SheetNote>{`${name} is offline. Open this again when it is back.`}</SheetNote>;
  else if (!canOperate(host)) body = <SheetNote>This device can only view; it cannot open tabs.</SheetNote>;
  else if (closed) body = <StartTab host={host} onOpened={launch.finish} />;
  else if (base === null) body = <SheetNote error>{`${name} has no tab to open one beside.`}</SheetNote>;
  else if (launch.ask)
    body = (
      <ConfirmProgram program={launch.ask} host={name} busy={launch.busy} onRun={() => void launch.go(launch.ask!)} onCancel={() => launch.setAsk(null)} />
    );
  else
    body = (
      <>
        <Group>
          <InfoRow first label="Opens on" value={`${name}, in Rift’s window`} />
          {cwd ? <InfoRow path label="Starts in" value={tildePath(cwd)} /> : null}
        </Group>
        <SectionLabel>Start with</SectionLabel>
        <ProgramRows kind="tab" disabled={launch.busy} onPick={launch.pick} />
        {launch.busy ? <Footnote>Opening…</Footnote> : null}
        <Footnote>The new tab opens beside this one on the computer, and this phone goes to it. Ask before running applies to the program’s name.</Footnote>
      </>
    );

  return (
    <SheetFrame title="New tab" subtitle={name || "Computer not found"} online={!!online} onClose={launch.close}>
      {body}
      {launch.error ? <ErrorText>{launch.error}</ErrorText> : null}
      {launch.opened !== null ? <Button label="Go to the new tab" onPress={() => launch.finish(launch.opened ?? undefined)} /> : null}
    </SheetFrame>
  );
}
