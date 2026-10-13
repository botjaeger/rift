import { Alert, Platform } from "react-native";

/** A destructive yes/no question; resolves true on the confirming button. */
export function confirmDestructive(title: string, message: string, action: string): Promise<boolean> {
  if (Platform.OS === "web") {
    // react-native-web's Alert has no buttons.
    return Promise.resolve(typeof window !== "undefined" && window.confirm(`${title}\n\n${message}`));
  }
  return new Promise((resolve) => {
    Alert.alert(
      title,
      message,
      [
        { text: "Cancel", style: "cancel", onPress: () => resolve(false) },
        { text: action, style: "destructive", onPress: () => resolve(true) },
      ],
      { cancelable: true, onDismiss: () => resolve(false) },
    );
  });
}
