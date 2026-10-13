import * as Clipboard from "expo-clipboard";
import { useRouter } from "expo-router";
import { useEffect, useState } from "react";
import { AccessibilityInfo, Linking, Platform, Pressable, StyleSheet, Text, TextInput, View } from "react-native";
import { controlKey } from "@/controlKeys";
import { ntfyUnsupported, type HostView } from "@/fleet";
import { NTFY_ROTATED, ntfyFooter, ntfyServerChanged, ntfySubscribeFallback, ntfySubscribeLink, pushFooter } from "@/notify";
import { pathLabel } from "@/protocol/relay";
import { authenticate, biometricSupport, fleet, useApp, type BiometricSupport } from "@/runtime";
import { MAX_DEVICE_NAME, NOTIFY_AFTER_CHOICES, type NotifyPolicy } from "@/stores";
import { Button } from "@/ui/Button";
import { confirmDestructive } from "@/ui/confirm";
import { formatAfter } from "@/ui/format";
import { ErrorText } from "@/ui/Notice";
import { ChoiceRow, Row, SwitchRow } from "@/ui/Rows";
import { Section, TabScreen } from "@/ui/Screen";
import { colors, mono, TOUCH } from "@/ui/theme";

const POLICIES: { value: NotifyPolicy; label: string }[] = [
  { value: "off", label: "Off" },
  { value: "failures", label: "Failed commands" },
  { value: "all", label: "Finished or failed commands" },
];

function DeviceName({ name }: { name: string }) {
  // The text being edited; null shows the saved name (which a load or another rename may change).
  const [draft, setDraft] = useState<string | null>(null);
  const save = () => {
    if (draft !== null) void fleet.updateSettings({ deviceName: draft });
    setDraft(null);
  };
  return (
    <Row first>
      <TextInput
        value={draft ?? name}
        onChangeText={setDraft}
        onFocus={() => setDraft(name)}
        onBlur={save}
        onSubmitEditing={save}
        maxLength={MAX_DEVICE_NAME}
        returnKeyType="done"
        placeholder="Device name"
        placeholderTextColor={colors.textMuted}
        accessibilityLabel="Device name"
        accessibilityHint="Shown on your computers when this device connects or runs a command"
        style={styles.input}
      />
    </Row>
  );
}

function PairedHost({ host, first }: { host: HostView; first: boolean }) {
  const [busy, setBusy] = useState(false);
  const unpair = async () => {
    const connected = host.conn.status === "open";
    const ok = await confirmDestructive(
      `Unpair ${host.record.name}?`,
      connected
        ? `${host.record.name} will forget this device, and this device will forget it. Pairing again needs a new code.`
        : `This device will forget ${host.record.name}. It is not connected now, so remove this device on the computer too (Settings › Remote access).`,
      "Unpair",
    );
    if (!ok) return;
    setBusy(true);
    await fleet.unpair(host.record.key);
  };
  const paths = [...host.record.addresses, ...(host.record.relay ? [pathLabel({ kind: "relay", url: host.record.relay })] : [])];
  return (
    <Row first={first} wrap>
      <View style={styles.hostText} accessible>
        <Text style={styles.hostName}>{host.record.name}</Text>
        <Text style={styles.hostMeta}>
          {host.record.permission === "view" ? "View only" : "Can run commands"} · {paths.join(", ")}
        </Text>
      </View>
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={`Unpair ${host.record.name}`}
        accessibilityHint="Asks first"
        accessibilityState={{ disabled: busy, busy }}
        disabled={busy}
        onPress={() => void unpair()}
        style={({ pressed }) => [styles.unpair, pressed && styles.pressed]}
      >
        <Text style={styles.unpairText}>{busy ? "Unpairing…" : "Unpair"}</Text>
      </Pressable>
    </Row>
  );
}

/**
 * Settings › Notifications › "Alerts through ntfy": a random topic on ntfy.sh (or a self-hosted
 * server) that the computers post alerts to; the ntfy app subscribed to it shows them.
 */
function Ntfy({ server, on, topic, hosts }: { server: string; on: boolean; topic: string | null; hosts: HostView[] }) {
  // The server being edited; null shows the saved one.
  const [draft, setDraft] = useState<string | null>(null);
  // A server that is not usable (kept until a usable one is saved), and other failures.
  const [serverError, setServerError] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const tell = (text: string) => {
    setNote(text);
    AccessibilityInfo.announceForAccessibility(text);
  };
  const saveServer = async () => {
    if (draft === null) return;
    const before = server;
    const ok = await fleet.setNtfyServer(draft);
    setServerError(ok ? null : "Enter an https server address, like ntfy.example.com. localhost and loopback addresses are not allowed.");
    if (!ok) return;
    setDraft(null);
    const now = fleet.state.settings.ntfyServer;
    if (now !== before) tell(ntfyServerChanged(now));
  };
  const copy = async (then: string): Promise<boolean> => {
    if (topic === null) return false;
    try {
      await Clipboard.setStringAsync(topic);
      setError(null);
      tell(then);
      return true;
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      return false;
    }
  };
  const subscribe = async () => {
    if (topic === null) return;
    setNote(null);
    try {
      await Linking.openURL(ntfySubscribeLink(server, topic));
    } catch {
      // Android: ntfy is not installed; iPhone: ntfy has no subscribe link
      const fallback = ntfySubscribeFallback(Platform.OS, server);
      if (fallback.copy) await copy(fallback.text);
      else tell(fallback.text);
    }
  };
  const rotate = async () => {
    const ok = await confirmDestructive(
      "Make a new topic?",
      "The ntfy app must subscribe to the new topic. Computers that get it stop posting to the old one; an offline one may still post there until it reconnects, and one unpaired while offline may keep posting there.",
      "New topic",
    );
    if (!ok) return;
    await fleet.rotateNtfyTopic();
    tell(NTFY_ROTATED);
  };
  const old = ntfyUnsupported(hosts);
  return (
    <Section footer={ntfyFooter(server)}>
      <SwitchRow
        first
        label="Alerts through ntfy"
        detail="While this app is closed, through the free ntfy app (no Apple or Google push setup needed)"
        value={on}
        onChange={(v) => {
          setNote(null);
          void fleet.setNtfy(v);
        }}
      />
      {on && topic !== null ? (
        <>
          <Row>
            <TextInput
              value={draft ?? server}
              onChangeText={setDraft}
              // an unusable draft stays to be corrected
              onFocus={() => setDraft((d) => d ?? server)}
              onBlur={() => void saveServer()}
              onSubmitEditing={() => void saveServer()}
              autoCapitalize="none"
              autoCorrect={false}
              keyboardType="url"
              returnKeyType="done"
              placeholder="https://ntfy.sh"
              placeholderTextColor={colors.textMuted}
              accessibilityLabel="ntfy server"
              accessibilityHint="ntfy.sh, or the address of your own ntfy server"
              style={styles.input}
            />
          </Row>
          <Row wrap>
            <View style={styles.hostText} accessible>
              <Text style={styles.hostMeta}>Topic</Text>
              <Text style={styles.topic} selectable>
                {topic}
              </Text>
            </View>
          </Row>
          <Row wrap>
            <Button
              label="Subscribe in ntfy"
              variant="primary"
              accessibilityHint="Opens the ntfy app on this topic"
              onPress={() => void subscribe()}
            />
            <Button label="Copy" accessibilityHint="Copies the topic" onPress={() => void copy("Topic copied")} />
            <Button
              label="New topic"
              accessibilityHint="Makes a new topic; the ntfy app must subscribe again. Asks first"
              onPress={() => void rotate()}
            />
          </Row>
          {old.map((h) => (
            <Row key={h.record.key}>
              <Text style={styles.reason}>Update Rift on {h.record.name} to get alerts there through ntfy.</Text>
            </Row>
          ))}
          {serverError || error || note ? (
            <Row>
              {serverError || error ? (
                <ErrorText style={styles.flex}>{serverError ?? error}</ErrorText>
              ) : (
                <Text style={styles.reason}>{note}</Text>
              )}
            </Row>
          ) : null}
        </>
      ) : null}
    </Section>
  );
}

/** Settings › Security: the app lock (hidden without biometrics while off, with the reason) and confirmation. */
function Security() {
  const s = useApp();
  const st = s.settings;
  const [support, setSupport] = useState<BiometricSupport | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    let live = true;
    void biometricSupport().then((r) => live && setSupport(r));
    return () => {
      live = false;
    };
  }, []);

  const setLock = async (on: boolean) => {
    setError(null);
    if (on) {
      // Prove it works before relying on it, so the lock can never shut the owner out.
      const r = await authenticate("Turn on the app lock");
      if (!r.ok) {
        if (r.message !== "Cancelled.") setError(r.message);
        return;
      }
    }
    await fleet.updateSettings({ biometricLock: on });
  };

  const lockAvailable = support?.label != null;
  const footer = [
    "Show the host, pane and command before every run.",
    lockAvailable
      ? "The lock asks on launch and after 5 minutes away, and hides the app's content in the app switcher. The device passcode works too."
      : null,
  ]
    .filter(Boolean)
    .join(" ");
  return (
    <Section title="Security" footer={footer}>
      <SwitchRow
        first
        label="Confirm every command"
        value={st.confirmEveryCommand}
        onChange={(v) => void fleet.updateSettings({ confirmEveryCommand: v })}
      />
      {/* A lock that is on stays switchable when the biometrics went away, so it can be turned off. */}
      {lockAvailable || st.biometricLock ? (
        <>
          <SwitchRow
            label={support?.label ?? "Require Face ID or fingerprint"}
            detail="On launch and after 5 minutes in the background"
            value={st.biometricLock}
            onChange={(v) => void setLock(v)}
          />
          {st.biometricLock ? (
            <SwitchRow
              label="Ask before running commands"
              detail={
                st.confirmEveryCommand
                  ? "Authenticate on the confirmation before Run"
                  : "Needs “Confirm every command”, which is off"
              }
              value={st.authBeforeRun}
              disabled={!st.confirmEveryCommand}
              onChange={(v) => void fleet.updateSettings({ authBeforeRun: v })}
            />
          ) : null}
        </>
      ) : null}
      {support && !lockAvailable ? (
        <Row>
          <Text style={styles.reason}>{support.reason}</Text>
        </Row>
      ) : null}
      {error ? (
        <Row>
          <ErrorText style={styles.flex}>{error}</ErrorText>
        </Row>
      ) : null}
    </Section>
  );
}

export default function SettingsScreen() {
  const router = useRouter();
  const s = useApp();
  const st = s.settings;
  return (
    <TabScreen title="Settings">
      <Section title="This device" footer="Shown on your computers when this device connects or runs a command.">
        <DeviceName name={s.deviceName} />
      </Section>

      <Security />

      <Section
        title="Notifications"
        footer={
          st.notifyPolicy === "off"
            ? "Only commands you asked about with “Notify me” notify."
            : `Commands that ran ${formatAfter(st.notifyAfterMs)} or longer, when you are not looking at ` +
              "that pane. A computer's own Notifications switch (on its page) turns them off for it. " +
              "“Notify me” always notifies."
        }
      >
        {POLICIES.map((p, i) => (
          <ChoiceRow
            key={p.value}
            first={i === 0}
            label={p.label}
            selected={st.notifyPolicy === p.value}
            onSelect={() => void fleet.updateSettings({ notifyPolicy: p.value })}
          />
        ))}
      </Section>

      {st.notifyPolicy !== "off" ? (
        <Section title="Notify after">
          {NOTIFY_AFTER_CHOICES.map((ms, i) => (
            <ChoiceRow
              key={ms}
              first={i === 0}
              label={formatAfter(ms)}
              selected={st.notifyAfterMs === ms}
              onSelect={() => void fleet.updateSettings({ notifyAfterMs: ms })}
            />
          ))}
        </Section>
      ) : null}

      <Section footer={pushFooter(s.push, st.ntfyEnabled)}>
        <SwitchRow
          first
          label="Show command in notification"
          detail="Command lines can contain secrets; off keeps them off the lock screen and out of push"
          value={st.showCommandInNotification}
          onChange={(v) => void fleet.updateSettings({ showCommandInNotification: v })}
        />
      </Section>

      <Ntfy server={st.ntfyServer} on={st.ntfyEnabled} topic={st.ntfyTopic} hosts={s.hosts} />

      <Section title="Keys" footer="The keys above the command line and under the terminal screen.">
        <Pressable
          accessibilityRole="button"
          accessibilityLabel="Control keys"
          accessibilityHint="Choose and order the keys of the control row"
          onPress={() => router.push("/control-keys")}
          style={({ pressed }) => [styles.addRow, pressed && styles.pressed]}
        >
          <Text style={styles.linkText}>Control keys</Text>
          <Text style={styles.linkDetail}>
            {st.controlKeys.map((id) => controlKey(id)?.label ?? id).join(" ") || "None"}
          </Text>
        </Pressable>
      </Section>

      <Section title="Paired desktops">
        {s.hosts.map((h, i) => (
          <PairedHost key={h.record.key} host={h} first={i === 0} />
        ))}
        <Pressable
          accessibilityRole="button"
          accessibilityLabel="Pair a desktop"
          accessibilityHint="Scan or paste the pairing code from Rift on your computer"
          onPress={() => router.push("/add")}
          style={({ pressed }) => [styles.addRow, s.hosts.length > 0 && styles.divider, pressed && styles.pressed]}
        >
          <Text style={styles.addText}>Pair a desktop…</Text>
        </Pressable>
      </Section>
    </TabScreen>
  );
}

const styles = StyleSheet.create({
  flex: { flex: 1 },
  input: { flex: 1, minHeight: TOUCH, color: colors.textBright, fontSize: 15 },
  hostText: { flexGrow: 1, flexShrink: 1, flexBasis: 200, gap: 2 },
  hostName: { color: colors.text, fontSize: 15 },
  hostMeta: { color: colors.textMuted, fontSize: 12, fontFamily: mono },
  topic: { color: colors.textBright, fontSize: 14, fontFamily: mono },
  // Destructive, but `error` is too dark for text on a surface (palette.ts): the caution token.
  unpair: { minHeight: TOUCH, minWidth: TOUCH, paddingHorizontal: 8, justifyContent: "center" },
  unpairText: { color: colors.warning, fontSize: 15, fontWeight: "500" },
  pressed: { opacity: 0.6 },
  addRow: { minHeight: TOUCH + 8, justifyContent: "center", paddingHorizontal: 14, paddingVertical: 8 },
  divider: { borderTopWidth: StyleSheet.hairlineWidth, borderTopColor: colors.hairline },
  addText: { color: colors.accent, fontSize: 15 },
  linkText: { color: colors.text, fontSize: 15 },
  linkDetail: { color: colors.textMuted, fontSize: 12, fontFamily: mono, marginTop: 2 },
  reason: { flex: 1, color: colors.textMuted, fontSize: 14, lineHeight: 20 },
});
