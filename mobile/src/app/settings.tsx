import Constants from "expo-constants";
import * as Updates from "expo-updates";
import { useRouter } from "expo-router";
import { useEffect, useRef, useState } from "react";
import { Platform, Pressable, Text, TextInput, View } from "react-native";
import { controlKey } from "@/controlKeys";
import { canOperate } from "@/fleet";
import { authenticate, biometricSupport, fleet, updateChecks, useApp, useUpdateChecks, type BiometricSupport } from "@/runtime";
import { MAX_DEVICE_NAME, NOTIFY_AFTER_CHOICES, type NotifyPolicy } from "@/stores";
import { APP_UPDATE } from "@/release";
import { appearanceLabel } from "@/appearance";
import { appVersionText } from "@/ui/format";
import { hostDot } from "@/ui/hostDot";
import { Icon } from "@/ui/Icon";
import { ErrorText } from "@/ui/Notice";
import { Button } from "@/ui/Button";
import { CardBox, CHEVRON, ChoiceRow, Group, NavRow, Row, SettingsPage, SwitchRow, ValueRow } from "@/ui/Rows";
import { SaveError } from "@/ui/SaveError";
import { hostMeta, notifyExample, ntfySummary, shortAfter, updateLine } from "@/ui/settingsText";
import { ui, makeStyles, useColors } from "@/ui/theme";
import { useNow } from "@/ui/useNow";
import { useActionScope } from "@/ui/useActionScope";

const POLICIES: { value: NotifyPolicy; label: string }[] = [
  { value: "off", label: "Nothing" },
  { value: "failures", label: "Failures" },
  { value: "all", label: "Everything" },
];

const NAME_START = { start: 0, end: 0 };

/** The Name field (a TextInput). */
type NameField = TextInput;

/** The widest the Name field grows to (pt): a longer name scrolls in it. */
const NAME_MAX = 150;

/**
 * This phone's Name (the Settings board: the value in Inter 15 and a ›): the field sits
 * right-aligned, as wide as the name; a tap anywhere in the row edits it. Saved when it loses
 * focus or on done.
 */
function DeviceName({ name }: { name: string }) {
  const colors = useColors();
  const styles = useStyles();
  const input = useRef<NameField>(null);
  // the name's own width, measured by an unseen copy: the field hugs it
  const [width, setWidth] = useState(0);
  // The text being edited; null shows the saved name (which a load or another rename may change).
  const [draft, setDraft] = useState<string | null>(null);
  // a name wider than the field shows its start, not its scrolled end, until it is edited
  const [editing, setEditing] = useState(false);
  const save = async () => {
    if (draft === null) return;
    if (await fleet.updateSettings({ deviceName: draft })) {
      setDraft((current) => current === draft ? null : current);
    }
  };
  const shown = draft ?? name;
  return (
    <Pressable accessible={false} onPress={() => input.current?.focus()}>
    <Row first>
      <View style={styles.text}>
        <Text style={styles.label}>Name</Text>
        <Text style={styles.detail}>Computers show it in toasts and logs</Text>
      </View>
      <Text style={[styles.nameInput, styles.measure]} numberOfLines={1} onLayout={(e) => setWidth(e.nativeEvent.layout.width)} importantForAccessibility="no-hide-descendants" accessibilityElementsHidden>
        {shown || "Phone name"}
      </Text>
      <TextInput
        ref={input}
        keyboardAppearance={colors.dark ? "dark" : "light"}
        value={shown}
        onChangeText={setDraft}
        selection={editing ? undefined : NAME_START}
        onFocus={() => {
          setEditing(true);
          setDraft((current) => current ?? name);
        }}
        onBlur={() => {
          setEditing(false);
          void save();
        }}
        onSubmitEditing={save}
        maxLength={MAX_DEVICE_NAME}
        returnKeyType="done"
        placeholder="Phone name"
        placeholderTextColor={colors.textMuted}
        accessibilityLabel="This phone's name"
        accessibilityHint="Shown on your computers when this phone connects or runs a command"
        style={[styles.nameInput, { width: Math.min(NAME_MAX, Math.ceil(width) + 4) }]}
      />
      <View style={styles.chevron} importantForAccessibility="no">
        <Icon name="forward" size={CHEVRON} color={colors.textMuted} />
      </View>
    </Row>
    </Pressable>
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

/** "Other commands [Failures ›]": the choices open under it. */
function OtherCommands({ value, onChange }: { value: NotifyPolicy; onChange: (v: NotifyPolicy) => void }) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <NavRow
        label="Other commands"
        value={POLICIES.find((p) => p.value === value)?.label}
        expanded={open}
        hint="Shows the choices"
        onPress={() => setOpen((o) => !o)}
      />
      {open ? (
        <View accessibilityRole="radiogroup" accessibilityLabel="Notify for other commands">
          {POLICIES.map((p) => (
            <ChoiceRow
              key={p.value}
              label={p.label}
              selected={value === p.value}
              onSelect={() => {
                onChange(p.value);
                setOpen(false);
              }}
            />
          ))}
        </View>
      ) : null}
    </>
  );
}

/** What the phone can unlock with: its words ("Face ID"), and whether the lock rows show. */
function useBiometrics(locked: boolean) {
  const [support, setSupport] = useState<BiometricSupport | null>(null);
  useEffect(() => {
    let live = true;
    void biometricSupport().then((r) => live && setSupport(r));
    return () => {
      live = false;
    };
  }, []);
  const available = support?.label != null;
  return {
    support,
    // a lock that is on stays switchable when the biometrics went away, so it can be turned off
    shown: available || locked,
    // the reason, once known, when there is no lock to offer
    reason: support && !available ? support.reason : null,
    // "Require Face ID" → "Face ID"
    bio: support?.label?.replace(/^Require /, "") ?? (Platform.OS === "ios" ? "Face ID or Touch ID" : "face unlock or fingerprint"),
  };
}

/** This phone's App lock row (hidden without biometrics while off, with the reason). */
function AppLock() {
  const styles = useStyles();
  const s = useApp();
  const scope = useActionScope();
  const st = s.settings;
  const { shown, reason, bio } = useBiometrics(st.biometricLock);
  const [error, setError] = useState<string | null>(null);

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

  return (
    <>
      {shown ? (
        <SwitchRow label="App lock" detail={bio} value={st.biometricLock} onChange={(v) => void setLock(v)} />
      ) : reason ? (
        <Row>
          <View style={styles.text}>
            <Text style={styles.label}>App lock</Text>
            <Text style={styles.reason}>{reason}</Text>
          </View>
        </Row>
      ) : null}
      {error ? (
        <Row>
          <ErrorText style={styles.flex1}>{error}</ErrorText>
        </Row>
      ) : null}
    </>
  );
}

/**
 * Running commands › Confirm with Face ID: needs the lock (without biometrics there is nothing to
 * confirm with). Off while it cannot apply, which is what runs (needsAuthBeforeRun).
 */
function ConfirmWithLock() {
  const s = useApp();
  const st = s.settings;
  const { shown, bio } = useBiometrics(st.biometricLock);
  // Only where a command is shown before it runs: with the lock and Ask before running on.
  const runReady = st.biometricLock && st.askBeforeRun;
  if (!shown) return null;
  return (
    <SwitchRow
      label={`Confirm with ${bio}`}
      detail={runReady ? "Before a confirmed command runs" : "Needs App lock and Ask before running"}
      value={runReady && st.authBeforeRun}
      disabled={!runReady}
      onChange={(v) => void fleet.updateSettings({ authBeforeRun: v })}
    />
  );
}

/**
 * The build's version and the over-the-air update it runs, the update checks (automatic, and
 * Check now) and the release notes; a downloaded update applies on restart.
 */
function About({ autoUpdate }: { autoUpdate: boolean }) {
  const styles = useStyles();
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
        <ValueRow first label="Version" value={text} />
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
                <Button label="Restart now" size="small" accessibilityHint="Restarts the app on the downloaded update" onPress={() => void Updates.reloadAsync()} />
              ) : (
                <Button
                  label={checking ? "Checking…" : "Check now"}
                  size="small"
                  disabled={checking}
                  accessibilityHint="Looks for an update and downloads it"
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
        <NavRow label="What's new" hint="The changes in each version of the app" onPress={() => router.push("/changelog")} />
      </CardBox>
    </Group>
  );
}

/**
 * Settings (the Settings board), sorted by what people look for: the computers, this phone (its
 * name, Appearance, App lock), notifications, running commands, panes, about.
 */
export default function SettingsScreen() {
  const colors = useColors();
  const styles = useStyles();
  const router = useRouter();
  const s = useApp();
  const st = s.settings;
  const example = s.hosts[0]?.record.name ?? "studio-mac";
  return (
    <SettingsPage title="Settings" notice={<SaveError />}>
      <Group title="Computers">
        <CardBox>
          {s.hosts.map((h, i) => (
            <NavRow
              key={h.record.key}
              first={i === 0}
              label={h.record.name}
              detail={hostMeta({ conn: h.conn, viewOnly: !canOperate(h) })}
              hint="Opens its settings"
              // the Portals card's dot
              leading={<View style={[styles.dot, { backgroundColor: colors[hostDot(h)] }]} />}
              onPress={() => router.push({ pathname: "/computer", params: { key: h.record.key } })}
            />
          ))}
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
      </Group>

      <Group title="This phone">
        <CardBox>
          <DeviceName name={s.deviceName} />
          <NavRow
            label="Appearance"
            value={appearanceLabel(st.appearance)}
            hint="Opens the theme choices"
            onPress={() => router.push("/appearance")}
          />
          <AppLock />
        </CardBox>
      </Group>

      <Group title="Notifications">
        <CardBox>
          <SwitchRow
            first
            label="Commands I ran from here"
            value={st.notifyMyCommands}
            onChange={(v) => void fleet.updateSettings({ notifyMyCommands: v })}
          />
          <OtherCommands value={st.notifyPolicy} onChange={(v) => void fleet.updateSettings({ notifyPolicy: v })} />
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
            detail="Shows the command first. ^C always sends at once."
            value={st.askBeforeRun}
            onChange={(v) => void fleet.updateSettings({ askBeforeRun: v })}
          />
          <ConfirmWithLock />
        </CardBox>
      </Group>

      <Group title="Panes">
        <CardBox>
          <SwitchRow
            first
            label="Wrap long lines"
            detail="Off: wide output scrolls sideways"
            value={st.wrapLines}
            onChange={(v) => void fleet.updateSettings({ wrapLines: v })}
          />
          <SwitchRow
            label="Keys under the command line"
            detail="Off: only under a program"
            value={st.showKeys}
            onChange={(v) => void fleet.updateSettings({ showKeys: v })}
          />
          <NavRow
            label="Keys"
            value={st.controlKeys.map((id) => controlKey(id)?.label ?? id).join(" ") || "None"}
            hint="Choose and order the keys in the key block"
            onPress={() => router.push("/control-keys")}
          />
        </CardBox>
      </Group>

      <About autoUpdate={st.autoUpdate} />
    </SettingsPage>
  );
}

const useStyles = makeStyles((colors) => ({
  flex1: { flex: 1 },
  text: { flex: 1, gap: 2 },
  // the list row's label and line (Rows.tsx)
  label: { fontFamily: ui(400), color: colors.textBright, fontSize: 16, lineHeight: 21 },
  detail: { fontFamily: ui(400), color: colors.textMuted, fontSize: 13, lineHeight: 17 },
  fresh: { color: colors.success },
  reason: { fontFamily: ui(400), flex: 1, color: colors.textMuted, fontSize: 13, lineHeight: 19 },
  dot: { width: 8, height: 8, borderRadius: 4 },
  // the row's value (Rows.tsx `value`): Inter 15, muted
  nameInput: {
    fontFamily: ui(400),
    minHeight: 44,
    paddingVertical: 0,
    paddingHorizontal: 0,
    color: colors.textMuted,
    fontSize: 15,
    textAlign: "right",
  },
  measure: { position: "absolute", opacity: 0, minHeight: 0, right: 0, maxWidth: NAME_MAX },
  chevron: { width: 14, alignItems: "center" },
}));
