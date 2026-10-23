import Constants from "expo-constants";
import * as Updates from "expo-updates";
import { useRouter } from "expo-router";
import { useEffect, useState } from "react";
import { Platform, StyleSheet, Text, TextInput, View } from "react-native";
import { controlKey } from "@/controlKeys";
import { canOperate } from "@/fleet";
import { authenticate, biometricSupport, fleet, updateChecks, useApp, useUpdateChecks, type BiometricSupport } from "@/runtime";
import { MAX_DEVICE_NAME, NOTIFY_AFTER_CHOICES, type NotifyPolicy } from "@/stores";
import { APP_UPDATE } from "@/release";
import { appVersionText } from "@/ui/format";
import { Icon } from "@/ui/Icon";
import { ErrorText } from "@/ui/Notice";
import { CardBox, ChoiceRow, Group, NavRow, Pill, Row, Segmented, SettingsPage, SwitchRow, ValueRow } from "@/ui/Rows";
import { SaveError } from "@/ui/SaveError";
import { hostMeta, notifyExample, ntfySummary, shortAfter, updateLine } from "@/ui/settingsText";
import { colors, ui } from "@/ui/theme";
import { useNow } from "@/ui/useNow";
import { useActionScope } from "@/ui/useActionScope";

const POLICIES: { value: NotifyPolicy; label: string }[] = [
  { value: "off", label: "Nothing" },
  { value: "failures", label: "Failures" },
  { value: "all", label: "Everything" },
];

/** "This phone's name": the field sits right-aligned in its row; saved when it loses focus or on done. */
function DeviceName({ name }: { name: string }) {
  // The text being edited; null shows the saved name (which a load or another rename may change).
  const [draft, setDraft] = useState<string | null>(null);
  const save = async () => {
    if (draft === null) return;
    if (await fleet.updateSettings({ deviceName: draft })) {
      setDraft((current) => current === draft ? null : current);
    }
  };
  return (
    <Row first>
      <View style={styles.text}>
        <Text style={styles.label}>This phone&apos;s name</Text>
        <Text style={styles.detail}>Computers show it in toasts and logs</Text>
      </View>
      <TextInput
        value={draft ?? name}
        onChangeText={setDraft}
        onFocus={() => setDraft((current) => current ?? name)}
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

/** "Only if it ran longer than [30 s ›]": the choices open under it. */
function LongerThan({ value, onChange }: { value: number; onChange: (ms: number) => void }) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <NavRow
        label="Only if it ran longer than"
        value={shortAfter(value)}
        expanded={open}
        hint="Shows the choices"
        onPress={() => setOpen((o) => !o)}
      />
      {open ? (
        <View accessibilityRole="radiogroup" accessibilityLabel="Only if it ran longer than">
          {NOTIFY_AFTER_CHOICES.map((ms) => (
            <ChoiceRow
              key={ms}
              label={shortAfter(ms)}
              selected={value === ms}
              onSelect={() => {
                onChange(ms);
                setOpen(false);
              }}
            />
          ))}
        </View>
      ) : null}
    </>
  );
}

/** Security: the app lock (hidden without biometrics while off, with the reason) and "Confirm commands". */
function Security() {
  const s = useApp();
  const scope = useActionScope();
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
    const current = scope.capture();
    setError(null);
    if (on) {
      // Prove it works before relying on it, so the lock can never shut the owner out.
      const r = await authenticate("Turn on the app lock");
      if (!current()) return;
      if (!r.ok) {
        if (r.message !== "Cancelled.") setError(r.message);
        return;
      }
    }
    await fleet.updateSettings({ biometricLock: on });
  };

  const lockAvailable = support?.label != null;
  const lockShown = lockAvailable || st.biometricLock;
  // "Require Face ID" → "Face ID"
  const bio = support?.label?.replace(/^Require /, "") ?? (Platform.OS === "ios" ? "Face ID or Touch ID" : "face unlock or fingerprint");
  // Only where a command is shown before it runs: with the lock and Ask before running on.
  const runReady = st.biometricLock && st.askBeforeRun;
  const reason = support && !lockAvailable;
  return (
    <Group title="Security">
      <CardBox>
        {/* A lock that is on stays switchable when the biometrics went away, so it can be turned off. */}
        {lockShown ? (
          <SwitchRow
            first
            label={`Lock with ${bio}`}
            detail="On opening, and after 5 minutes away"
            value={st.biometricLock}
            onChange={(v) => void setLock(v)}
          />
        ) : null}
        {/* Needs the lock: without biometrics there is nothing to confirm with. Off while it
            cannot apply, which is what runs (needsAuthBeforeRun). */}
        {lockShown ? (
          <SwitchRow
            label={`Confirm commands with ${bio}`}
            detail={runReady ? "Before a confirmed command runs" : `Needs Lock with ${bio} and Ask before running`}
            value={runReady && st.authBeforeRun}
            disabled={!runReady}
            onChange={(v) => void fleet.updateSettings({ authBeforeRun: v })}
          />
        ) : null}
        {reason ? (
          <Row first={!lockShown}>
            <Text style={styles.reason}>{support.reason}</Text>
          </Row>
        ) : null}
        {error ? (
          <Row>
            <ErrorText style={styles.flex1}>{error}</ErrorText>
          </Row>
        ) : null}
      </CardBox>
    </Group>
  );
}

/**
 * The build's version and the over-the-air update it runs, the update checks (automatic, and
 * Check now) and the release notes; a downloaded update applies on restart.
 */
function About({ autoUpdate }: { autoUpdate: boolean }) {
  const router = useRouter();
  const { currentlyRunning, isUpdatePending } = Updates.useUpdates();
  const check = useUpdateChecks();
  const now = useNow(5000, check.state === "none");
  const text = appVersionText(Constants.expoConfig?.version ?? "?", {
    enabled: Updates.isEnabled,
    embedded: currentlyRunning.isEmbeddedLaunch,
    createdAt: currentlyRunning.createdAt ?? null,
    update: APP_UPDATE,
  });
  const checking = check.state === "checking";
  const line = updateLine(check, isUpdatePending, now);
  return (
    <Group title="About">
      <CardBox>
        <ValueRow first label="Version" value={text} mono />
        {updateChecks.enabled ? (
          <>
            <Row>
              <View style={styles.text}>
                <Text style={styles.label}>Updates</Text>
                <Text style={[styles.detail, line.fresh && styles.fresh]} accessibilityLiveRegion="polite">
                  {line.text}
                </Text>
              </View>
              {isUpdatePending ? (
                <Pill label="Restart now" height={34} hint="Restarts the app on the downloaded update" onPress={() => void Updates.reloadAsync()} />
              ) : (
                <Pill
                  label={checking ? "Checking…" : "Check now"}
                  height={34}
                  disabled={checking}
                  hint="Looks for an update and downloads it"
                  onPress={() => void updateChecks.run()}
                />
              )}
            </Row>
            <SwitchRow
              label="Check automatically"
              detail="At launch and every 15 minutes"
              value={autoUpdate}
              onChange={(v) => void fleet.updateSettings({ autoUpdate: v })}
            />
          </>
        ) : null}
        <NavRow label="What’s New" accessibilityLabel="What's New" hint="The changes in each version of the app" onPress={() => router.push("/changelog")} />
      </CardBox>
    </Group>
  );
}

/** Settings (the Settings board). */
export default function SettingsScreen() {
  const router = useRouter();
  const s = useApp();
  const st = s.settings;
  const example = s.hosts[0]?.record.name ?? "studio-mac";
  return (
    <SettingsPage title="Settings" notice={<SaveError />}>
      <Group title="Computers">
        <CardBox>
          {s.hosts.map((h, i) => {
            const os = h.welcome?.os ?? h.record.os ?? null;
            return (
              <NavRow
                key={h.record.key}
                first={i === 0}
                label={h.record.name}
                detail={hostMeta({ os, conn: h.conn, viewOnly: !canOperate(h) })}
                hint="Opens its settings"
                leading={<View style={[styles.dot, { backgroundColor: h.conn.status === "open" ? colors.success : colors.textMuted }]} />}
                onPress={() => router.push({ pathname: "/computer", params: { key: h.record.key } })}
              />
            );
          })}
          <NavRow
            first={s.hosts.length === 0}
            label="Pair a computer"
            accent
            chevron={false}
            leading={<Icon name="add" size={18} color={colors.accent} />}
            hint="Opens the camera to scan a code"
            onPress={() => router.push("/scan")}
          />
        </CardBox>
        <CardBox>
          <DeviceName name={s.deviceName} />
        </CardBox>
      </Group>

      <Group title="Notifications" footer="Nothing notifies for the pane on screen. Questions always notify.">
        <CardBox>
          <SwitchRow
            first
            label="My commands"
            detail="When a command you ran from this phone finishes"
            value={st.notifyMyCommands}
            onChange={(v) => void fleet.updateSettings({ notifyMyCommands: v })}
          />
          <View style={styles.policy}>
            <Text style={styles.label}>Other commands on your computers</Text>
            <Segmented
              label="Notify for other commands"
              options={POLICIES}
              value={st.notifyPolicy}
              onChange={(v) => void fleet.updateSettings({ notifyPolicy: v })}
            />
          </View>
          {st.notifyPolicy !== "off" ? (
            <LongerThan value={st.notifyAfterMs} onChange={(ms) => void fleet.updateSettings({ notifyAfterMs: ms })} />
          ) : null}
          <SwitchRow
            label="Show the command"
            detail={notifyExample(example, st.showCommandInNotification)}
            value={st.showCommandInNotification}
            onChange={(v) => void fleet.updateSettings({ showCommandInNotification: v })}
          />
          <NavRow
            label="While the app is closed"
            detail="Alerts through the ntfy app"
            value={ntfySummary(st.ntfyEnabled)}
            onPress={() => router.push("/settings-ntfy")}
          />
        </CardBox>
      </Group>

      <Group title="Running commands">
        <CardBox>
          <SwitchRow
            first
            label="Ask before running"
            detail="Shows the command before it runs. Keys like ⌃C always send at once."
            value={st.askBeforeRun}
            onChange={(v) => void fleet.updateSettings({ askBeforeRun: v })}
          />
        </CardBox>
      </Group>

      <Security />

      <Group title="Panes">
        <CardBox>
          <SwitchRow
            first
            label="Wrap long lines"
            detail="Off: wide output scrolls sideways. Each pane can change it."
            value={st.wrapLines}
            onChange={(v) => void fleet.updateSettings({ wrapLines: v })}
          />
          <SwitchRow
            label="Keys above the command line"
            detail="Off: only while a program has the full screen"
            value={st.showKeys}
            onChange={(v) => void fleet.updateSettings({ showKeys: v })}
          />
          <NavRow
            label="Choose keys"
            detail={st.controlKeys.map((id) => controlKey(id)?.label ?? id).join(" ") || "None"}
            mono
            hint="Choose and order the keys above the command line"
            onPress={() => router.push("/control-keys")}
          />
        </CardBox>
      </Group>

      <About autoUpdate={st.autoUpdate} />
    </SettingsPage>
  );
}

const styles = StyleSheet.create({
  flex1: { flex: 1 },
  text: { flex: 1, gap: 2 },
  label: { fontFamily: ui(400), color: colors.textBright, fontSize: 15 },
  detail: { fontFamily: ui(400), color: colors.textMuted, fontSize: 12, lineHeight: 16 },
  fresh: { color: colors.success },
  reason: { fontFamily: ui(400), flex: 1, color: colors.textMuted, fontSize: 13, lineHeight: 19 },
  dot: { width: 8, height: 8, borderRadius: 4 },
  policy: {
    gap: 10,
    paddingHorizontal: 14,
    paddingVertical: 12,
    borderTopWidth: 1,
    borderTopColor: colors.hairline,
  },
  nameInput: {
    fontFamily: ui(400),
    width: 130,
    minHeight: 44,
    paddingVertical: 0,
    color: colors.textMuted,
    fontSize: 15,
    textAlign: "right",
  },
});
