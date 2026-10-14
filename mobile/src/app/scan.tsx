import { CameraView, useCameraPermissions, type BarcodeScanningResult } from "expo-camera";
import { useRouter } from "expo-router";
import { useRef, useState } from "react";
import { Linking, ScrollView, StyleSheet, Text, useWindowDimensions, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { parsePairLink } from "@/pairing";
import { Button } from "@/ui/Button";
import { PageHeader } from "@/ui/Screen";
import { colors, radius, ui } from "@/ui/theme";

/** The four accent corners of the viewfinder. */
function Corners() {
  return (
    <>
      <View style={[styles.corner, styles.tl]} />
      <View style={[styles.corner, styles.tr]} />
      <View style={[styles.corner, styles.bl]} />
      <View style={[styles.corner, styles.br]} />
    </>
  );
}

/** "Pair a computer" (the Pair board): the camera reads the code Rift shows. */
export default function ScanScreen() {
  const router = useRouter();
  const insets = useSafeAreaInsets();
  const window = useWindowDimensions();
  // the board's 300 pt frame, smaller on a narrow or short phone (320 × 568) so the copy and
  // its button stay above the paste button; the page scrolls if they still do not fit
  const frame = Math.max(200, Math.min(300, window.width - 48, window.height - 420));
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

  const granted = permission?.granted ?? false;
  const blocked = permission !== null && !permission.granted && !permission.canAskAgain;
  const close = () => (router.canGoBack() ? router.back() : router.replace("/"));

  return (
    <View style={styles.fill}>
      <PageHeader title="Pair a computer" back={close} backIcon="close" />
      <ScrollView style={styles.flex} contentContainerStyle={styles.body}>
        <View style={[styles.viewfinder, { width: frame, height: frame }]} accessible={!granted} accessibilityLabel="Camera view with a frame for the code">
          {granted ? (
            <CameraView
              style={StyleSheet.absoluteFill}
              facing="back"
              barcodeScannerSettings={{ barcodeTypes: ["qr"] }}
              onBarcodeScanned={onScanned}
              accessibilityLabel="Camera viewfinder"
            />
          ) : null}
          <Corners />
        </View>
        <View style={styles.copy}>
          <Text style={styles.title} accessibilityRole="header">
            {permission && !granted ? (blocked ? "Camera access is off" : "Scan with the camera") : "Point at the code in Rift"}
          </Text>
          <Text style={[styles.text, wrongCode && granted && styles.wrong]} accessibilityLiveRegion="polite">
            {permission && !granted
              ? blocked
                ? "Rift may not use the camera. Turn it on in Settings, or paste the pairing link instead."
                : "Rift uses the camera only to read the pairing code on your computer's screen."
              : wrongCode
                ? "That QR code is not a Rift pairing code."
                : "On your computer: Rift › Settings › Remote access › Pair a phone."}
          </Text>
          {permission && !granted ? (
            blocked ? (
              <Button label="Open Settings" variant="primary" onPress={() => void Linking.openSettings()} />
            ) : (
              <Button label="Allow camera" variant="primary" onPress={() => void requestPermission()} />
            )
          ) : null}
        </View>
      </ScrollView>
      <View style={[styles.actions, { paddingBottom: Math.max(40, insets.bottom + 6) }]}>
        <Button label="Paste a pairing link" style={styles.paste} onPress={() => router.replace("/add")} />
      </View>
    </View>
  );
}

const C = 44;
const styles = StyleSheet.create({
  fill: { flex: 1, backgroundColor: colors.camera },
  flex: { flex: 1 },
  body: { alignItems: "center", paddingTop: 28, paddingBottom: 8, paddingHorizontal: 24, gap: 28 },
  viewfinder: { borderRadius: 16, backgroundColor: colors.viewfinder, overflow: "hidden" },
  corner: { position: "absolute", width: C, height: C, borderColor: colors.accent },
  tl: { left: 22, top: 22, borderLeftWidth: 3, borderTopWidth: 3, borderTopLeftRadius: 10 },
  tr: { right: 22, top: 22, borderRightWidth: 3, borderTopWidth: 3, borderTopRightRadius: 10 },
  bl: { left: 22, bottom: 22, borderLeftWidth: 3, borderBottomWidth: 3, borderBottomLeftRadius: 10 },
  br: { right: 22, bottom: 22, borderRightWidth: 3, borderBottomWidth: 3, borderBottomRightRadius: 10 },
  copy: { maxWidth: 310, gap: 10, alignItems: "stretch" },
  title: { fontFamily: ui(600), color: colors.textBright, fontSize: 20, textAlign: "center" },
  text: { fontFamily: ui(400), color: colors.textMuted, fontSize: 14, lineHeight: 21, textAlign: "center" },
  wrong: { color: colors.warning },
  actions: { paddingTop: 16, paddingHorizontal: 24 },
  paste: { minHeight: 48, borderRadius: radius.button },
});
