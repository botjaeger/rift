import * as Clipboard from "expo-clipboard";
import { useState } from "react";
import { AccessibilityInfo, Linking, Platform, StyleSheet, Text, TextInput, View } from "react-native";
import { ntfyUnsupported } from "@/fleet";
import { NTFY_ROTATED, ntfyFooter, ntfyServerChanged, ntfySubscribeFallback, ntfySubscribeLink, pushFooter } from "@/notify";
import { fleet, useApp } from "@/runtime";
import { confirmDestructive } from "@/ui/confirm";
import { ErrorText } from "@/ui/Notice";
import { CardBox, Group, Pill, Row, SettingsPage, SwitchRow } from "@/ui/Rows";
import { colors, mono, ui } from "@/ui/theme";

/**
 * Settings › While the app is closed › "Alerts through ntfy": a random topic on ntfy.sh (or a
 * self-hosted server) that the computers post alerts to; the ntfy app subscribed to it shows them.
 */
export default function NtfyScreen() {
  const s = useApp();
  const st = s.settings;
  const { ntfyServer: server, ntfyEnabled: on, ntfyTopic: topic } = st;
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
  const old = ntfyUnsupported(s.hosts);
  const footer = on ? ntfyFooter(server, st.showCommandInNotification) : pushFooter(s.push, false);
  return (
    <SettingsPage title="While the app is closed">
      <Group footer={footer}>
        <CardBox>
          <SwitchRow
            first
            label="Alerts through ntfy"
            detail="Free, with the ntfy app. Works while Rift is closed."
            value={on}
            onChange={(v) => {
              setNote(null);
              void fleet.setNtfy(v);
            }}
          />
        </CardBox>
      </Group>

      {on && topic !== null ? (
        <Group title="Where alerts go">
          <CardBox>
            <Row first>
              <Text style={[styles.label, styles.text]}>Server</Text>
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
            </Row>
            <View style={styles.topicBlock} accessible accessibilityLabel={`Topic, keep it secret, ${topic}`}>
              <Text style={styles.label}>
                Topic <Text style={styles.secret}>· keep it secret</Text>
              </Text>
              <Text style={styles.topic} selectable>
                {topic}
              </Text>
            </View>
          </CardBox>
          <View style={styles.buttons}>
            <Pill kind="primary" height={50} label="Subscribe in ntfy" hint="Opens the ntfy app on this topic" onPress={() => void subscribe()} />
            <View style={styles.pair}>
              <Pill label="Copy topic" hint="Copies the topic" onPress={() => void copy("Topic copied")} style={styles.flex1} />
              <Pill
                kind="caution"
                label="New topic…"
                hint="Makes a new topic; the ntfy app must subscribe again. Asks first"
                onPress={() => void rotate()}
                style={styles.flex1}
              />
            </View>
          </View>
          {old.map((h) => (
            <Text key={h.record.key} style={styles.reason}>
              Update Rift on {h.record.name} to get alerts there through ntfy.
            </Text>
          ))}
          {serverError || error ? <ErrorText>{serverError ?? error}</ErrorText> : note ? <Text style={styles.reason}>{note}</Text> : null}
          <Text style={styles.foot}>A new topic stops alerts until the ntfy app subscribes to it.</Text>
        </Group>
      ) : null}
    </SettingsPage>
  );
}

const styles = StyleSheet.create({
  flex1: { flex: 1 },
  text: { flex: 1 },
  label: { fontFamily: ui(400), color: colors.textBright, fontSize: 15 },
  secret: { fontFamily: ui(400), fontSize: 12, color: colors.warning },
  reason: { fontFamily: ui(400), color: colors.textMuted, fontSize: 13, lineHeight: 19, paddingHorizontal: 16 },
  foot: { fontFamily: ui(400), color: colors.textDim, fontSize: 12, lineHeight: 17, paddingHorizontal: 16 },
  serverInput: {
    fontFamily: mono,
    width: 170,
    minHeight: 44,
    paddingVertical: 0,
    color: colors.textMuted,
    fontSize: 14,
    textAlign: "right",
  },
  topicBlock: {
    gap: 6,
    paddingHorizontal: 14,
    paddingVertical: 12,
    borderTopWidth: 1,
    borderTopColor: colors.hairline,
  },
  topic: {
    fontFamily: mono,
    color: colors.text,
    fontSize: 13,
    paddingVertical: 10,
    paddingHorizontal: 12,
    borderRadius: 10,
    backgroundColor: colors.chrome,
    overflow: "hidden",
  },
  buttons: { gap: 8, marginTop: 4 },
  pair: { flexDirection: "row", gap: 8 },
});
