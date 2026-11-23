import { useLocalSearchParams, useRouter } from "expo-router";
import { useMemo, useState } from "react";
import { View } from "react-native";
import { canOperate, hasFeature } from "@/fleet";
import { useApp } from "@/runtime";
import { tunnelAvailable } from "@/tunnel";
import { openPort } from "@/tunnel/open";
import { localLinksIn } from "@/tunnel/url";
import { Button } from "@/ui/Button";
import { Field } from "@/ui/Field";
import { Footnote, SheetNote } from "@/ui/NewSheet";
import { CardBox, Group, NavRow } from "@/ui/Rows";
import { SheetFrame } from "@/ui/SheetFrame";

/** How far back in the open pane the sheet looks for pages it printed. */
const RECENT_BLOCKS = 6;

/**
 * Open a local page (from the computer menu): a page served on the computer's `localhost`
 * opens in the browser sheet through the tunnel (feature `tunnel`, src/tunnel). The pages the
 * open pane printed come first (a dev server's "Local: http://localhost:5173/"), then any port.
 */
export default function LocalPageSheet() {
  const { key } = useLocalSearchParams<{ key: string }>();
  const router = useRouter();
  const s = useApp();
  const host = s.hosts.find((h) => h.record.key === key);
  const name = host?.record.name ?? "";
  const online = host?.conn.status === "open";
  const [port, setPort] = useState("");
  const pane = s.pane?.host === key ? s.pane : null;
  const found = useMemo(() => {
    if (!pane) return [];
    const blocks = pane.state?.blocks.slice(-RECENT_BLOCKS).reverse() ?? [];
    const screen = pane.screen?.lines.map((l) => l.map((r) => r[0]).join("")).join("\n") ?? "";
    return localLinksIn([screen, ...blocks.map((b) => b.output)]);
  }, [pane]);
  const n = Number(port);
  const valid = /^\d{1,5}$/.test(port) && n >= 1 && n <= 65535;
  const open = (p: number, path = "/") => {
    router.back();
    openPort(key, p, path);
  };

  let body;
  let footer = null;
  if (!host) body = <SheetNote error>Computer not found.</SheetNote>;
  else if (!online) body = <SheetNote>{`${name} is offline. Open this again when it is back.`}</SheetNote>;
  else if (!canOperate(host)) body = <SheetNote>This device can only view; it cannot open the computer's pages.</SheetNote>;
  else if (!hasFeature(host, "tunnel")) body = <SheetNote>{`Update Rift on ${name} to open its local pages here.`}</SheetNote>;
  else if (!tunnelAvailable) body = <SheetNote>Update this app to open the computer's local pages.</SheetNote>;
  else {
    body = (
      <>
        {found.length > 0 ? (
          <Group title="In this pane">
            <CardBox>
              {found.map((l, i) => (
                <NavRow key={l.port} first={i === 0} label={`localhost:${l.port}`} value={l.path === "/" ? undefined : l.path} hint={`Opens ${l.url}`} onPress={() => open(l.port, l.path)} />
              ))}
            </CardBox>
          </Group>
        ) : null}
        <Group title="Port">
          <View>
            <Field
              icon="globe"
              label="Port"
              value={port}
              onChangeText={(t) => setPort(t.replace(/\D/g, "").slice(0, 5))}
              placeholder="5173"
              keyboardType="number-pad"
              returnKeyType="go"
              onSubmitEditing={() => valid && open(n)}
            />
          </View>
        </Group>
        <Footnote>{`Opens http://localhost:PORT of ${name} in the browser. It reaches what runs on ${name}, nothing else on its network.`}</Footnote>
      </>
    );
    footer = <Button label={valid ? `Open localhost:${n}` : "Open"} variant="primary" disabled={!valid} onPress={() => open(n)} />;
  }

  return (
    <SheetFrame board="localPage" title="Open a local page" subtitle={name ? `On ${name}` : "Computer not found"} onClose={() => router.back()} footer={footer}>
      {body}
    </SheetFrame>
  );
}
