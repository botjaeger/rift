import { useLocalSearchParams } from "expo-router";
import { canOperate } from "@/fleet";
import { tildePath } from "@/chips";
import { useApp } from "@/runtime";
import { Button } from "@/ui/Button";
import { ErrorText } from "@/ui/Notice";
import { basePane, newTabCwd } from "@/ui/newTab";
import { ConfirmProgram, Footnote, ProgramRows, SheetFrame, SheetNote } from "@/ui/NewSheet";
import { CardBox, Group, ValueRow } from "@/ui/Rows";
import { canStartTab, StartTab } from "@/ui/StartTab";
import { findPane, tabTitle } from "@/topology";
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
  // the tab it opens beside ("On studio-mac, beside project1")
  const beside = host && base !== null ? findPane(host.windows, base) : null;

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
        {/* the folder as one row (the new tab starts where the tab beside it is) */}
        {cwd ? (
          <CardBox>
            <ValueRow first path label="Folder" value={tildePath(cwd)} />
          </CardBox>
        ) : null}
        <Group title="Start with">
          <ProgramRows kind="tab" disabled={launch.busy} onPick={launch.pick} />
        </Group>
        {launch.busy ? <Footnote>Opening…</Footnote> : null}
        <Footnote>The new tab opens beside this one on the computer, and this phone goes to it. Ask before running applies to the program’s name.</Footnote>
      </>
    );

  return (
    <SheetFrame
      title="New tab"
      subtitle={name ? `On ${name}${beside ? `, beside ${tabTitle(beside.tab)}` : ""}${online ? "" : " · offline"}` : "Computer not found"}
      onClose={launch.close}
    >
      {body}
      {launch.error ? <ErrorText>{launch.error}</ErrorText> : null}
      {launch.opened !== null ? <Button label="Go to the new tab" onPress={() => launch.finish(launch.opened ?? undefined)} /> : null}
    </SheetFrame>
  );
}
