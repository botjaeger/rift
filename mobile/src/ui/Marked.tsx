import { useMemo, type ReactNode } from "react";
import { Text } from "react-native";
import { markParts, type MarkPart } from "@/paneView";
import { makeStyles } from "./theme";

/** The pieces as a Text's children: the matches in the mark style, the rest plain strings. */
export function markChildren(parts: readonly MarkPart[], style: object): ReactNode[] {
  return parts.map((p, i) =>
    p.mark ? (
      <Text key={i} style={style}>
        {p.text}
      </Text>
    ) : (
      p.text
    ),
  );
}

/** The mark's style: the warning tint behind the match, its text bright (test/contrast.test.ts). */
export const useMarkStyle = makeStyles((colors) => ({
  mark: { backgroundColor: colors.tints.match, color: colors.textBright },
}));

/**
 * `text` inside a Text with each occurrence of the Search output `query` marked (the New Blocks
 * board); the plain text when there is no query or no match.
 */
export function Marked({ text, query }: { text: string; query?: string }) {
  const styles = useMarkStyle();
  const parts = useMemo(() => (query ? markParts(text, query) : null), [text, query]);
  return <>{parts ? markChildren(parts, styles.mark) : text}</>;
}
