import Constants from "expo-constants";
import { useLocalSearchParams, useRouter } from "expo-router";
import { useMemo } from "react";
import { parseChangelog, releaseKey, releasesSince } from "@/changelog";
import { CHANGELOG } from "@/changelog.data";
import { APP_UPDATE } from "@/release";
import { Button } from "@/ui/Button";
import { SheetFrame } from "@/ui/SheetFrame";
import { ReleaseList, releaseLabel } from "@/ui/ReleaseNotes";
import { makeStyles } from "@/ui/theme";

/**
 * What's new, once after an update or a new build (the root opens it, useWhatsNew): the notes
 * of every release since the one last seen, and one Done pinned to the sheet's foot. `seen` is
 * that release's key ("" for none known).
 */
export default function WhatsNewScreen() {
  const styles = useStyles();
  const router = useRouter();
  const { seen } = useLocalSearchParams<{ seen?: string }>();
  const releases = useMemo(() => {
    const all = parseChangelog(CHANGELOG);
    const current = releaseKey(Constants.expoConfig?.version ?? "", APP_UPDATE);
    const since = releasesSince(all, typeof seen === "string" ? seen : "", current);
    // the sheet is only opened with something new; a stale link shows the newest notes
    return since.length > 0 ? since : all.slice(0, 1);
  }, [seen]);
  return (
    <SheetFrame
      board="whatsNew"
      title="What's new"
      subtitle={releaseLabel(releases[0]!)}
      onClose={() => router.back()}
      bodyStyle={styles.body}
      footer={<Button label="Done" variant="primary" onPress={() => router.back()} />}
    >
      <ReleaseList releases={releases} named={false} sheet />
    </SheetFrame>
  );
}

// the board: 18 pt in
const useStyles = makeStyles(() => ({
  body: { paddingHorizontal: 18 },
}));
