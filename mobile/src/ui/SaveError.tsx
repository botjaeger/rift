import { StyleSheet } from "react-native";
import { useApp } from "@/runtime";
import { ErrorText } from "./Notice";

/** A settings change this phone could not save (it was put back), until a change saves. */
export function SaveError() {
  const error = useApp().settingsError;
  return error ? <ErrorText style={styles.error}>{error}</ErrorText> : null;
}

const styles = StyleSheet.create({
  error: { marginHorizontal: 16, marginBottom: 8 },
});
