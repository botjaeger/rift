import { useCallback, useEffect, useRef, useState } from "react";
import { AccessibilityInfo, StyleSheet, Text } from "react-native";
import { colors, ui } from "./theme";

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
    toast: text ? (
      <Text style={styles.toast} accessibilityLiveRegion="polite">
        {text}
      </Text>
    ) : null,
  };
}

const styles = StyleSheet.create({
  toast: {
    alignSelf: "center",
    fontFamily: ui(500),
    color: colors.bg,
    fontSize: 14,
    paddingVertical: 10,
    paddingHorizontal: 16,
    borderRadius: 20,
    overflow: "hidden",
    backgroundColor: colors.textBright,
  },
});
