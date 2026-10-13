import { useRouter } from "expo-router";
import { useEffect, useState } from "react";
import { Pressable, StyleSheet, Text, TextInput, View } from "react-native";
import type { HostView } from "@/fleet";
import { pushFooter } from "@/notify";
import { pathLabel } from "@/protocol/relay";
import { authenticate, biometricSupport, fleet, useApp, type BiometricSupport } from "@/runtime";
import { MAX_DEVICE_NAME } from "@/stores";
import { confirmDestructive } from "@/ui/confirm";
import { ErrorText } from "@/ui/Notice";
import { Row, SwitchRow } from "@/ui/Rows";
import { Section, TabScreen } from "@/ui/Screen";
import { colors, mono, TOUCH } from "@/ui/theme";

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
        footer={pushFooter(s.push)}
      >
        <SwitchRow
          first
          label="Command finished or failed"
          detail="Commands that ran 30 seconds or longer, when you are not looking at that pane"
          value={st.notifyDone}
          onChange={(v) => void fleet.updateSettings({ notifyDone: v })}
        />
        <SwitchRow
          label="Show command in notification"
          detail="Command lines can contain secrets; off keeps them off the lock screen and out of push"
          value={st.showCommandInNotification}
          onChange={(v) => void fleet.updateSettings({ showCommandInNotification: v })}
        />
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
  // Destructive, but `error` is too dark for text on a surface (palette.ts): the caution token.
  unpair: { minHeight: TOUCH, minWidth: TOUCH, paddingHorizontal: 8, justifyContent: "center" },
  unpairText: { color: colors.warning, fontSize: 15, fontWeight: "500" },
  pressed: { opacity: 0.6 },
  addRow: { minHeight: TOUCH + 8, justifyContent: "center", paddingHorizontal: 14, paddingVertical: 8 },
  divider: { borderTopWidth: StyleSheet.hairlineWidth, borderTopColor: colors.hairline },
  addText: { color: colors.accent, fontSize: 15 },
  reason: { flex: 1, color: colors.textMuted, fontSize: 14, lineHeight: 20 },
});
