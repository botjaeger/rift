import { useLocalSearchParams, useRouter } from "expo-router";
import { canOperate } from "@/fleet";
import { tildePath } from "@/chips";
import { useApp } from "@/runtime";
import { Button } from "@/ui/Button";
import { ErrorText } from "@/ui/Notice";
import { basePane, newTabCwd } from "@/ui/newTab";
import { ConfirmProgram, ConfirmProgramButtons, Footnote, ProgramRows, SheetNote } from "@/ui/NewSheet";
import { SheetFrame } from "@/ui/SheetFrame";
import { CardBox, Group, NavRow } from "@/ui/Rows";
import { canStartTab, StartTab } from "@/ui/StartTab";
import { findPane, tabTitle } from "@/topology";
import { useLaunch } from "@/ui/useLaunch";

/**
 * The New tab sheet (the New tab boards), opened from the Tabs sheet or a computer's card on
 * Portals (`from=home`): what the new tab starts with, a shell or a coding agent typed into it.
 * Folder is where it starts, the folder of the tab beside it: a tap opens Change folder for that
 * tab, whose cd moves the start too. With Rift closed on the computer it is the Open a tab step
 * instead (a shell only: the computer keeps no program to start for a phone while Rift is
 * closed). View-only devices get no start.
 */
export default function NewTabSheet() {
  const { key, from } = useLocalSearchParams<{ key: string; from?: string }>();
  const router = useRouter();
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
  let footer = null;
  if (!host) body = <SheetNote error>Computer not found.</SheetNote>;
  else if (!online) body = <SheetNote>{`${name} is offline. Open this again when it is back.`}</SheetNote>;
  else if (!canOperate(host)) body = <SheetNote>This device can only view; it cannot open tabs.</SheetNote>;
  else if (closed) body = <StartTab host={host} onOpened={launch.finish} />;
  else if (base === null) body = <SheetNote error>{`${name} has no tab to open one beside.`}</SheetNote>;
  else if (launch.ask) {
    body = <ConfirmProgram program={launch.ask} host={name} />;
    footer = <ConfirmProgramButtons busy={launch.busy} onRun={() => void launch.go(launch.ask!)} onCancel={() => launch.setAsk(null)} />;
  } else
    body = (
      <>
        {/* the folder as one row (the new tab starts where the tab beside it is) */}
        {cwd ? (
          <CardBox>
            <NavRow
              first
              path
              label="Folder"
              value={tildePath(cwd)}
              hint={`Opens Change folder for ${beside ? tabTitle(beside.tab) : "the tab beside it"}`}
              disabled={launch.busy}
              onPress={() => router.push({ pathname: "/chips", params: { key, pane: String(base), menu: "dirs" } })}
            />
          </CardBox>
        ) : null}
        <Group title="Start with">
          <ProgramRows kind="tab" disabled={launch.busy} onPick={launch.pick} />
        </Group>
        {launch.busy ? <Footnote>Opening…</Footnote> : null}
      </>
    );
  if (launch.opened !== null) footer = <Button label="Go to the new tab" onPress={() => launch.finish(launch.opened ?? undefined)} />;

  return (
    <SheetFrame
      board="newTab"
      title="New tab"
      subtitle={name ? `On ${name}${beside ? `, beside ${tabTitle(beside.tab)}` : ""}${online ? "" : " · offline"}` : "Computer not found"}
      onClose={launch.close}
      footer={footer}
    >
      {body}
      {launch.error ? <ErrorText>{launch.error}</ErrorText> : null}
    </SheetFrame>
  );
}
