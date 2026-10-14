import { Text, type StyleProp, type TextStyle } from "react-native";
import { mono, ui } from "./theme";

/** A question with its paths in the mono face, as the boards draw them ("Apply this edit to `src/x.rs`?"). */
export function PromptText({ text, style, monoSize }: { text: string; style: StyleProp<TextStyle>; monoSize: number }) {
  // a path is any word with a slash; its trailing punctuation stays in the text face
  const parts = text.split(/(\S*[/\\]\S*?)(?=[?:.,]?(?:\s|$))/);
  return (
    <Text style={[{ fontFamily: ui(400) }, style]}>
      {parts.map((p, i) =>
        i % 2 === 1 ? (
          <Text key={i} style={{ fontFamily: mono, fontSize: monoSize }}>
            {p}
          </Text>
        ) : (
          p
        ),
      )}
    </Text>
  );
}
