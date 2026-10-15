import { useRouter } from "expo-router";
import { useMemo } from "react";
import { ScrollView, StyleSheet } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { parseChangelog, releaseKey } from "@/changelog";
import { CHANGELOG } from "@/changelog.data";
import { ReleaseCard } from "@/ui/ReleaseNotes";
import { PageHeader } from "@/ui/Screen";
import { colors } from "@/ui/theme";

/** Settings › About › What's New: every release's notes, newest first. */
export default function ChangelogScreen() {
  const router = useRouter();
  const insets = useSafeAreaInsets();
  const releases = useMemo(() => parseChangelog(CHANGELOG), []);
  return (
    <>
      <PageHeader
        title="What's New"
        back={() => (router.canGoBack() ? router.back() : router.replace("/settings"))}
        backLabel="Back"
      />
      <ScrollView style={styles.fill} contentContainerStyle={[styles.content, { paddingBottom: insets.bottom + 24 }]}>
        {releases.map((r) => (
          <ReleaseCard key={releaseKey(r.version, r.update)} release={r} />
        ))}
      </ScrollView>
    </>
  );
}

const styles = StyleSheet.create({
  fill: { flex: 1, backgroundColor: colors.bg },
  content: { paddingHorizontal: 12, paddingTop: 8, gap: 12, maxWidth: 720, width: "100%", alignSelf: "center" },
});
