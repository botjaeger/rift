import Constants from "expo-constants";
import * as Clipboard from "expo-clipboard";
import * as Updates from "expo-updates";
import { useRouter } from "expo-router";
import { useEffect, useState } from "react";
import { AccessibilityInfo, Linking, Platform, Pressable, StyleSheet, Text, TextInput, View } from "react-native";
import { controlKey } from "@/controlKeys";
import { ntfyUnsupported, type HostView } from "@/fleet";
import { NTFY_ROTATED, ntfyFooter, ntfyServerChanged, ntfySubscribeFallback, ntfySubscribeLink, pushFooter } from "@/notify";
import { authenticate, biometricSupport, fleet, useApp, type BiometricSupport } from "@/runtime";
import { MAX_DEVICE_NAME, NOTIFY_AFTER_CHOICES, type NotifyPolicy } from "@/stores";
import { Button } from "@/ui/Button";
import { confirmDestructive } from "@/ui/confirm";
import { appVersionText, formatAfter, osName } from "@/ui/format";
import { Icon } from "@/ui/Icon";
import { ErrorText } from "@/ui/Notice";
import { ChoiceRow, InfoRow, Row, SwitchRow } from "@/ui/Rows";
import { Section, TabScreen } from "@/ui/Screen";
import { colors, mono, radius, TOUCH, ui } from "@/ui/theme";

const POLICIES: { value: NotifyPolicy; label: string }[] = [
  { value: "off", label: "Nothing" },
  { value: "failures", label: "Failures" },
  { value: "all", label: "Everything" },
];

/** "30 s", "1 min", "5 min" (the Settings board's picker). */
const shortAfter = (ms: number) => (ms < 60_000 ? `${Math.round(ms / 1000)} s` : `${Math.round(ms / 60_000)} min`);

/** Nothing | Failures | Everything: one radio group, the choice on the accent. */
function NotifyFor({ value, onChange }: { value: NotifyPolicy; onChange: (v: NotifyPolicy) => void }) {
  return (
    <View style={styles.notifyFor}>
      <Text style={styles.rowLabel}>Notify for</Text>
      <View style={styles.segments} accessibilityRole="radiogroup" accessibilityLabel="Notify for">
        {POLICIES.map((p) => {
          const on = p.value === value;
          return (
            <Pressable
              key={p.value}
              accessibilityRole="radio"
              accessibilityLabel={p.label}
              accessibilityState={{ checked: on }}
              // 36 pt segments, 44 pt to the touch (the group's padding and border)
              hitSlop={{ top: 4, bottom: 4 }}
              onPress={() => onChange(p.value)}
              style={({ pressed }) => [styles.segment, on && styles.segmentOn, pressed && styles.pressed]}
            >
              <Text style={[styles.segmentText, on && styles.segmentTextOn]}>{p.label}</Text>
            </Pressable>
          );
        })}
      </View>
    </View>
  );
}

/** "Only if it ran longer than [30 s ▾]": the choices open under it. */
function LongerThan({ value, onChange }: { value: number; onChange: (ms: number) => void }) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <Row>
        <View style={styles.rowText}>
          <Text style={styles.rowLabel}>Only if it ran longer than</Text>
          <Text style={styles.rowDetail}>Short ones never notify</Text>
        </View>
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={`Only if it ran longer than ${formatAfter(value)}`}
          accessibilityHint="Shows the choices"
          accessibilityState={{ expanded: open }}
          onPress={() => setOpen((o) => !o)}
          hitSlop={{ top: 4, bottom: 4 }}
          style={({ pressed }) => [styles.picker, pressed && styles.pressed]}
        >
          <Text style={styles.pickerText}>{shortAfter(value)} ▾</Text>
        </Pressable>
      </Row>
      {open
        ? NOTIFY_AFTER_CHOICES.map((ms) => (
            <ChoiceRow
              key={ms}
              label={formatAfter(ms)}
              selected={value === ms}
              onSelect={() => {
                onChange(ms);
                setOpen(false);
              }}
            />
          ))
        : null}
    </>
  );
}

function DeviceName({ name, first }: { name: string; first: boolean }) {
  // The text being edited; null shows the saved name (which a load or another rename may change).
  const [draft, setDraft] = useState<string | null>(null);
  const save = () => {
    if (draft !== null) void fleet.updateSettings({ deviceName: draft });
    setDraft(null);
  };
  return (
    // wraps: on a 320 pt phone the field beside the label was too narrow and cut the name's first letter
    <Row first={first} wrap>
      <Text style={styles.rowLabel}>This phone&apos;s name</Text>
      <TextInput
        value={draft ?? name}
        onChangeText={setDraft}
        onFocus={() => setDraft(name)}
        onBlur={save}
        onSubmitEditing={save}
        maxLength={MAX_DEVICE_NAME}
        returnKeyType="done"
        placeholder="Phone name"
        placeholderTextColor={colors.textMuted}
        accessibilityLabel="This phone's name"
        accessibilityHint="Shown on your computers when this phone connects or runs a command"
        style={styles.nameInput}
      />
    </Row>
  );
}

function PairedHost({ host, first }: { host: HostView; first: boolean }) {
  const router = useRouter();
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
  const os = host.welcome?.os ?? host.record.os;
  const meta = [os ? osName(os) : null, host.record.permission === "view" ? "view only" : "can run commands"]
    .filter(Boolean)
    .join(" · ");
  return (
    <View style={[styles.host, !first && styles.divider]}>
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={`${host.record.name}, ${meta}`}
        accessibilityHint="Opens its tabs and alerts"
        onPress={() => router.push({ pathname: "/host/[key]", params: { key: host.record.key } })}
        style={({ pressed }) => [styles.rowText, pressed && styles.pressed]}
      >
        <Text style={styles.rowLabel}>{host.record.name}</Text>
        <Text style={styles.rowDetail}>{meta}</Text>
      </Pressable>
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={`Unpair ${host.record.name}`}
        accessibilityHint="Asks first"
        accessibilityState={{ disabled: busy, busy }}
        disabled={busy}
        onPress={() => void unpair()}
        style={({ pressed }) => [styles.unpair, pressed && styles.pressed]}
      >
        {/* Destructive, but red text is below 4.5:1 on a surface (palette.ts): the caution token. */}
        <Text style={styles.unpairText}>{busy ? "Unpairing…" : "Unpair"}</Text>
      </Pressable>
    </View>
  );
}

/**
 * Settings › While the app is closed › "Alerts through ntfy": a random topic on ntfy.sh (or a
 * self-hosted server) that the computers post alerts to; the ntfy app subscribed to it shows them.
 */
function Ntfy({
  server,
  on,
  topic,
  hosts,
  footer,
}: {
  server: string;
  on: boolean;
  topic: string | null;
  hosts: HostView[];
  footer: string;
}) {
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
    <Section title="While the app is closed" footer={footer}>
      <SwitchRow
        first
        label="Alerts through ntfy"
        detail="Free, with the ntfy app"
        value={on}
        onChange={(v) => {
          setNote(null);
          void fleet.setNtfy(v);
        }}
      />
      {on && topic !== null ? (
        <View style={[styles.ntfy, styles.divider]}>
          <View style={styles.field}>
            <Text style={styles.fieldLabel}>Server</Text>
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
              style={styles.serverInput}
            />
          </View>
          <View style={styles.field} accessible>
            <Text style={styles.fieldLabel}>Topic (keep it secret)</Text>
            <Text style={styles.topic} selectable>
              {topic}
            </Text>
          </View>
          <View style={styles.ntfyButtons}>
            <Button
              label="Subscribe in ntfy"
              variant="primary"
              size="small"
              accessibilityHint="Opens the ntfy app on this topic"
              onPress={() => void subscribe()}
              style={styles.flex2}
            />
            <Button label="Copy" size="small" accessibilityHint="Copies the topic" onPress={() => void copy("Topic copied")} style={styles.flex1} />
            <Button
              label="New"
              size="small"
              accessibilityHint="Makes a new topic; the ntfy app must subscribe again. Asks first"
              onPress={() => void rotate()}
              style={styles.flex1}
            />
          </View>
          {old.map((h) => (
            <Text key={h.record.key} style={styles.reason}>
              Update Rift on {h.record.name} to get alerts there through ntfy.
            </Text>
          ))}
          {serverError || error ? <ErrorText>{serverError ?? error}</ErrorText> : note ? <Text style={styles.reason}>{note}</Text> : null}
        </View>
      ) : null}
    </Section>
  );
}

/** Settings › Security: the app lock (hidden without biometrics while off, with the reason) and "Run with Face ID". */
function Security({ deviceName }: { deviceName: string }) {
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
  // "Require Face ID" → "Face ID"
  const bio = support?.label?.replace(/^Require /, "") ?? (Platform.OS === "ios" ? "Face ID or Touch ID" : "face unlock or fingerprint");
  return (
    <Section title="Security">
      {/* A lock that is on stays switchable when the biometrics went away, so it can be turned off. */}
      {lockAvailable || st.biometricLock ? (
        <SwitchRow
          first
          label={`Lock with ${bio}`}
          detail="On opening, and after 5 minutes away"
          value={st.biometricLock}
          onChange={(v) => void setLock(v)}
        />
      ) : null}
      {/* Only where a command is shown before it runs: with the lock and Ask before running on. */}
      {st.biometricLock && st.askBeforeRun ? (
        <SwitchRow
          label={`Run with ${bio}`}
          detail="Before a command you confirm runs, and before keys that submit a line"
          value={st.authBeforeRun}
          onChange={(v) => void fleet.updateSettings({ authBeforeRun: v })}
        />
      ) : null}
      {support && !lockAvailable ? (
        <Row first={!st.biometricLock}>
          <Text style={styles.reason}>{support.reason}</Text>
        </Row>
      ) : null}
      {error ? (
        <Row>
          <ErrorText style={styles.flex1}>{error}</ErrorText>
        </Row>
      ) : null}
      <DeviceName
        name={deviceName}
        first={!(lockAvailable || st.biometricLock) && !(support && !lockAvailable) && !error}
      />
    </Section>
  );
}

/** Settings (the Settings board). */
export default function SettingsScreen() {
  const router = useRouter();
  const s = useApp();
  const st = s.settings;
  const example = s.hosts[0]?.record.name ?? "studio-mac";
  return (
    <TabScreen title="Settings">
      <Section title="Running commands">
        <SwitchRow
          first
          label="Ask before running"
          detail="Off: a command runs as soon as you tap Run. Keys like ^C always send at once."
          value={st.askBeforeRun}
          onChange={(v) => void fleet.updateSettings({ askBeforeRun: v })}
        />
        <SwitchRow
          label="Notify me when my commands finish"
          detail="Every command you run from this phone, however long it takes. Tap the bell on a running block to change one."
          value={st.notifyMyCommands}
          onChange={(v) => void fleet.updateSettings({ notifyMyCommands: v })}
        />
      </Section>

      <Section
        title="Other commands on your computers"
        footer="Nothing notifies for the pane you are looking at. “Needs you” alerts show the question a program asks, whatever “Show the command” says."
      >
        <NotifyFor value={st.notifyPolicy} onChange={(v) => void fleet.updateSettings({ notifyPolicy: v })} />
        {st.notifyPolicy !== "off" ? (
          <LongerThan value={st.notifyAfterMs} onChange={(ms) => void fleet.updateSettings({ notifyAfterMs: ms })} />
        ) : null}
        <SwitchRow
          label="Show the command"
          detail={`Off: “A command failed on ${example} (exit 1)”`}
          value={st.showCommandInNotification}
          onChange={(v) => void fleet.updateSettings({ showCommandInNotification: v })}
        />
      </Section>

      <Ntfy
        server={st.ntfyServer}
        on={st.ntfyEnabled}
        topic={st.ntfyTopic}
        hosts={s.hosts}
        footer={st.ntfyEnabled ? ntfyFooter(st.ntfyServer, st.showCommandInNotification) : pushFooter(s.push, false)}
      />

      <Section title="Output">
        <SwitchRow
          first
          label="Wrap long lines"
          detail="Off: each block scrolls sideways, lines stay as the computer prints them"
          value={st.wrapLines}
          onChange={(v) => void fleet.updateSettings({ wrapLines: v })}
        />
        <SwitchRow
          label="Show keys above the command line"
          detail="Off: the row shows only while a program has the full screen."
          value={st.showKeys}
          onChange={(v) => void fleet.updateSettings({ showKeys: v })}
        />
        <Pressable
          accessibilityRole="button"
          accessibilityLabel="Keys"
          accessibilityHint="Choose and order the keys above the command line"
          onPress={() => router.push("/control-keys")}
          style={({ pressed }) => [styles.link, styles.divider, pressed && styles.pressed]}
        >
          <View style={styles.rowText}>
            <Text style={styles.rowLabel}>Keys</Text>
            <Text style={styles.keysPreview}>
              {st.controlKeys.map((id) => controlKey(id)?.label ?? id).join(" ") || "None"}
            </Text>
          </View>
          <Icon name="forward" size={14} color={colors.textMuted} />
        </Pressable>
      </Section>

      <Security deviceName={s.deviceName} />

      <Section title="Paired computers" footer="To add one: in Rift on that computer, Settings › Remote access › Pair a phone.">
        {s.hosts.length === 0 ? (
          <Row first>
            <Text style={styles.reason}>None yet.</Text>
          </Row>
        ) : null}
        {s.hosts.map((h, i) => (
          <PairedHost key={h.record.key} host={h} first={i === 0} />
        ))}
      </Section>

      <About />
    </TabScreen>
  );
}

/** The build's version and the over-the-air update it runs; a downloaded update applies on restart. */
function About() {
  const { currentlyRunning, isUpdatePending } = Updates.useUpdates();
  const text = appVersionText(Constants.expoConfig?.version ?? "?", {
    enabled: Updates.isEnabled,
    embedded: currentlyRunning.isEmbeddedLaunch,
    createdAt: currentlyRunning.createdAt ?? null,
  });
  return (
    <Section title="About" footer={isUpdatePending ? "An update is downloaded and applies when the app restarts." : undefined}>
      <InfoRow label="Version" first>
        <Text style={styles.version}>{text}</Text>
      </InfoRow>
      {isUpdatePending ? (
        <Row>
          <Button label="Restart now" size="small" accessibilityHint="Restarts the app on the downloaded update" onPress={() => void Updates.reloadAsync()} />
        </Row>
      ) : null}
    </Section>
  );
}

const styles = StyleSheet.create({
  flex1: { flex: 1 },
  flex2: { flex: 2 },
  pressed: { opacity: 0.6 },
  divider: { borderTopWidth: 1, borderTopColor: colors.hairline },
  rowText: { flex: 1, gap: 3 },
  rowLabel: { fontFamily: ui(400), color: colors.textBright, fontSize: 15 },
  rowDetail: { fontFamily: ui(400), color: colors.textMuted, fontSize: 12, lineHeight: 17 },
  reason: { fontFamily: ui(400), flex: 1, color: colors.textMuted, fontSize: 13, lineHeight: 19 },
  version: { fontFamily: mono, color: colors.textBright, fontSize: 13 },
  // Notify for
  notifyFor: { paddingVertical: 12, paddingHorizontal: 14, gap: 10 },
  segments: {
    flexDirection: "row",
    padding: 3,
    borderRadius: radius.button,
    backgroundColor: colors.bg,
    borderWidth: 1,
    borderColor: colors.hairline,
  },
  segment: { flex: 1, minHeight: 36, borderRadius: radius.small, alignItems: "center", justifyContent: "center" },
  segmentOn: { backgroundColor: colors.accent },
  segmentText: { fontFamily: ui(400), color: colors.textMuted, fontSize: 13 },
  segmentTextOn: { fontFamily: ui(600), color: colors.bg },
  picker: {
    minHeight: 36,
    paddingHorizontal: 12,
    borderRadius: radius.small,
    borderWidth: 1,
    borderColor: colors.borderStrong,
    justifyContent: "center",
  },
  pickerText: { fontFamily: ui(400), color: colors.text, fontSize: 14 },
  // ntfy
  ntfy: { paddingTop: 10, paddingBottom: 12, paddingHorizontal: 14, gap: 10 },
  field: { gap: 4 },
  fieldLabel: { fontFamily: ui(400), color: colors.textMuted, fontSize: 12 },
  serverInput: {
    fontFamily: ui(400),
    height: 40,
    paddingHorizontal: 10,
    borderRadius: radius.small,
    borderWidth: 1,
    borderColor: colors.borderStrong,
    backgroundColor: colors.bg,
    color: colors.textBright,
    fontSize: 14,
  },
  topic: { color: colors.textBright, fontSize: 13, fontFamily: mono },
  ntfyButtons: { flexDirection: "row", gap: 8 },
  // Output
  link: { minHeight: 60, flexDirection: "row", alignItems: "center", gap: 12, paddingHorizontal: 14, paddingVertical: 12 },
  keysPreview: { color: colors.textMuted, fontSize: 12, fontFamily: mono },
  // Security
  nameInput: {
    fontFamily: ui(400),
    flexGrow: 1,
    flexBasis: 160,
    minWidth: 160,
    height: 36,
    paddingVertical: 0,
    paddingHorizontal: 10,
    borderRadius: radius.small,
    borderWidth: 1,
    borderColor: colors.borderStrong,
    backgroundColor: colors.bg,
    color: colors.textBright,
    fontSize: 14,
    textAlign: "right",
  },
  // Paired computers
  host: { minHeight: 56, flexDirection: "row", alignItems: "center", gap: 12, paddingVertical: 8, paddingLeft: 14, paddingRight: 6 },
  unpair: { minHeight: TOUCH, paddingHorizontal: 12, justifyContent: "center" },
  unpairText: { fontFamily: ui(400), color: colors.warning, fontSize: 14 },
});
