import { useCallback, useEffect, useRef, useState } from "react";
import { AccessibilityInfo, Text, View } from "react-native";
import { ui, makeStyles } from "./theme";

/**
 * The toast (the New Blocks board): a dark pill with a rim and a shadow, white Inter 500 13. The
 * pane screen places it over the tab row; the sheets get one from `useToast`.
 */
export function Toast({ text }: { text: string }) {
  const styles = useStyles();
  return (
    <View style={styles.shadow}>
      <Text style={styles.toast} accessibilityLiveRegion="polite">
        {text}
      </Text>
    </View>
  );
}

const useStyles = makeStyles((colors) => ({
  toast: {
    alignSelf: "center",
    fontFamily: ui(500),
    color: colors.textBright,
    fontSize: 13,
    lineHeight: 18,
    paddingVertical: 8,
    paddingHorizontal: 14,
    borderRadius: 18,
    borderWidth: 1,
    borderColor: colors.borderStrong,
    overflow: "hidden",
    backgroundColor: colors.menu,
  },
  // the shadow is the wrapper's: a Text that clips its corners clips its shadow too
  shadow: {
    alignSelf: "center",
    borderRadius: 18,
    shadowColor: "#000000",
    shadowOpacity: 0.6,
    shadowRadius: 12,
    shadowOffset: { width: 0, height: 8 },
    elevation: 8,
  },
}));

/** A line that goes after a moment and is announced, as the pane screen's toast (for the sheets). */
export function useToast(): { say: (text: string) => void; toast: React.ReactNode } {
  const [text, setText] = useState("");
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  useEffect(() => () => clearTimeout(timer.current), []);
  const say = useCallback((next: string) => {
    clearTimeout(timer.current);
    setText(next);
    AccessibilityInfo.announceForAccessibility(next);
    timer.current = setTimeout(() => setText(""), 1600);
  }, []);
  return {
    say,
    toast: text ? <Toast text={text} /> : null,
  };
}
