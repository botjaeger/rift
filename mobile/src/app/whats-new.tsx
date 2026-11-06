import Constants from "expo-constants";
import { useLocalSearchParams, useRouter } from "expo-router";
import { useMemo } from "react";
import { ScrollView, Text, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { parseChangelog, releaseKey, releasesSince } from "@/changelog";
import { CHANGELOG } from "@/changelog.data";
import { APP_UPDATE } from "@/release";
import { Button } from "@/ui/Button";
import { ReleaseList, releaseLabel } from "@/ui/ReleaseNotes";
import { SheetHeader } from "@/ui/SheetHeader";
import { ui, makeStyles } from "@/ui/theme";

/**
 * What's New, once after an update or a new build (the root opens it, useWhatsNew): the notes
 * of every release since the one last seen. `seen` is that release's key ("" for none known).
 */
export default function WhatsNewScreen() {
  const styles = useStyles();
  const router = useRouter();
  const insets = useSafeAreaInsets();
  const { seen } = useLocalSearchParams<{ seen?: string }>();
  const releases = useMemo(() => {
    const all = parseChangelog(CHANGELOG);
    const current = releaseKey(Constants.expoConfig?.version ?? "", APP_UPDATE);
    const since = releasesSince(all, typeof seen === "string" ? seen : "", current);
    // the sheet is only opened with something new; a stale link shows the newest notes
    return since.length > 0 ? since : all.slice(0, 1);
  }, [seen]);
  return (
    <ScrollView style={styles.fill} contentContainerStyle={{ paddingBottom: Math.max(36, insets.bottom + 2) }}>
      <SheetHeader title="What’s new" subtitle={releaseLabel(releases[0]!)} onClose={() => router.back()} />
      <View style={styles.content}>
        <Text style={styles.lead}>Rift was updated. Here is what changed.</Text>
        <ReleaseList releases={releases} named={false} />
        <View style={styles.buttons}>
          <Button label="All changes" style={styles.button} onPress={() => router.replace("/changelog")} />
          <Button label="Done" variant="primary" style={styles.button} onPress={() => router.back()} />
        </View>
      </View>
    </ScrollView>
  );
}

const useStyles = makeStyles((colors) => ({
  fill: { backgroundColor: colors.surface },
  content: { paddingTop: 4, paddingHorizontal: 20, gap: 16, maxWidth: 560, width: "100%", alignSelf: "center" },
  lead: { fontFamily: ui(400), color: colors.textMuted, fontSize: 14, lineHeight: 20 },
  buttons: { flexDirection: "row", flexWrap: "wrap", gap: 10 },
  button: { flexGrow: 1, flexBasis: 120 },
}));
