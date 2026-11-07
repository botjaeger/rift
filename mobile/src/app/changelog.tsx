import { useRouter } from "expo-router";
import { useMemo } from "react";
import { ScrollView } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { parseChangelog } from "@/changelog";
import { CHANGELOG } from "@/changelog.data";
import { ReleaseList } from "@/ui/ReleaseNotes";
import { PageHeader } from "@/ui/Screen";
import { makeStyles } from "@/ui/theme";

/** Settings › About › What's New: every release's notes, newest first. */
export default function ChangelogScreen() {
  const styles = useStyles();
  const router = useRouter();
  const insets = useSafeAreaInsets();
  const releases = useMemo(() => parseChangelog(CHANGELOG), []);
  return (
    <>
      <PageHeader
        title="What's new"
        back={() => (router.canGoBack() ? router.back() : router.replace("/settings"))}
        backLabel="Back"
      />
      <ScrollView style={styles.fill} contentContainerStyle={[styles.content, { paddingBottom: insets.bottom + 24 }]}>
        <ReleaseList releases={releases} named />
      </ScrollView>
    </>
  );
}

const useStyles = makeStyles((colors) => ({
  fill: { flex: 1, backgroundColor: colors.bg },
  // the board: 8 pt over the first release, 18 pt in
  content: { paddingHorizontal: 18, paddingTop: 8, maxWidth: 720, width: "100%", alignSelf: "center" },
}));
