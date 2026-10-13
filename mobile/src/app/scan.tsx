import { CameraView, useCameraPermissions, type BarcodeScanningResult } from "expo-camera";
import { useRouter } from "expo-router";
import { useRef, useState } from "react";
import { Linking, StyleSheet, Text, View } from "react-native";
import { parsePairLink } from "@/pairing";
import { Button } from "@/ui/Button";
import { colors, radius } from "@/ui/theme";

export default function ScanScreen() {
  const router = useRouter();
  const [permission, requestPermission] = useCameraPermissions();
  const [wrongCode, setWrongCode] = useState(false);
  const handled = useRef(false);

  const onScanned = ({ data }: BarcodeScanningResult) => {
    if (handled.current) return;
    if (!parsePairLink(data)) {
      setWrongCode(true);
      return;
    }
    handled.current = true;
    router.replace({ pathname: "/pair", params: { link: data.trim() } });
  };

  if (!permission) return <View style={styles.fill} />;

  if (!permission.granted) {
    const blocked = !permission.canAskAgain;
    return (
      <View style={[styles.fill, styles.message]}>
        <Text style={styles.title} accessibilityRole="header">
          {blocked ? "Camera access is off" : "Scan with the camera"}
        </Text>
        <Text style={styles.text}>
          {blocked
            ? "Rift may not use the camera. Turn it on in Settings, or paste the pairing link instead."
            : "Rift uses the camera only to read the pairing code on your computer's screen."}
        </Text>
        {blocked ? (
          <Button label="Open Settings" variant="primary" onPress={() => void Linking.openSettings()} />
        ) : (
          <Button label="Allow camera" variant="primary" onPress={() => void requestPermission()} />
        )}
        <Button label="Paste the link instead" onPress={() => router.replace("/add")} />
      </View>
    );
  }

  return (
    <View style={styles.fill}>
      <CameraView
        style={styles.fill}
        facing="back"
        barcodeScannerSettings={{ barcodeTypes: ["qr"] }}
        onBarcodeScanned={onScanned}
        accessibilityLabel="Camera viewfinder"
      />
      <View style={styles.overlay} pointerEvents="box-none">
        <Text style={[styles.hint, wrongCode && styles.hintError]} accessibilityLiveRegion="polite">
          {wrongCode ? "That QR code is not a Rift pairing code." : "Point the camera at the pairing code on your computer."}
        </Text>
        <Button label="Paste the link instead" onPress={() => router.replace("/add")} />
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  fill: { flex: 1, backgroundColor: colors.bg },
  message: { padding: 24, gap: 14, justifyContent: "center", maxWidth: 560, width: "100%", alignSelf: "center" },
  title: { color: colors.textBright, fontSize: 20, fontWeight: "600" },
  text: { color: colors.textMuted, fontSize: 14, lineHeight: 20 },
  overlay: { position: "absolute", left: 16, right: 16, bottom: 32, gap: 12 },
  hint: {
    color: colors.textBright,
    fontSize: 14,
    textAlign: "center",
    backgroundColor: "#000000b3",
    paddingHorizontal: 12,
    paddingVertical: 8,
    borderRadius: radius.control,
    overflow: "hidden",
  },
  hintError: { color: colors.warning },
});
