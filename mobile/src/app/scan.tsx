import { CameraView, useCameraPermissions, type BarcodeScanningResult } from "expo-camera";
import { useRouter } from "expo-router";
import { useRef, useState } from "react";
import { Linking, ScrollView, StyleSheet, Text, useWindowDimensions, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { parsePairLink } from "@/pairing";
import { Button } from "@/ui/Button";
import { PageHeader } from "@/ui/Screen";
import { ui, makeStyles, PaletteProvider, paletteOf } from "@/ui/theme";
import { DARK_DEFAULT } from "@/ui/palette";
import { StatusBar } from "expo-status-bar";

/** The four accent corners of the viewfinder. */
function Corners() {
  const styles = useStyles();
  return (
    <>
      <View style={[styles.corner, styles.tl]} />
      <View style={[styles.corner, styles.tr]} />
      <View style={[styles.corner, styles.bl]} />
      <View style={[styles.corner, styles.br]} />
    </>
  );
}

/**
 * "Pair a computer" (the Pair board): the camera reads the code Rift shows. The camera's black
 * page keeps Dark Default's colours whatever the app's theme (palette.ts, CAMERA_TEXT).
 */
export default function ScanScreen() {
  return (
    <PaletteProvider palette={paletteOf(DARK_DEFAULT)}>
      <StatusBar style="light" />
      <ScanPage />
    </PaletteProvider>
  );
}

function ScanPage() {
  const styles = useStyles();
  const router = useRouter();
  const insets = useSafeAreaInsets();
  const window = useWindowDimensions();
  // the board's 264 pt frame, smaller on a narrow or short phone (320 × 568) so the copy and
  // its buttons stay in sight; the page scrolls if they still do not fit
  const frame = Math.max(200, Math.min(264, window.width - 48, window.height - 420));
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
      <PageHeader title="Pair a computer" back={close} />
      {/* the New Pairing board: the viewfinder, the heading on the left, the path under it, and at the
          bottom Allow camera full width over the paste link */}
      <ScrollView style={styles.flex} contentContainerStyle={[styles.body, { paddingBottom: Math.max(16, insets.bottom) }]}>
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
        <Text style={styles.title} accessibilityRole="header">
          {permission && !granted && blocked ? "Camera access is off" : "Scan the code on your computer"}
        </Text>
        <Text style={[styles.text, wrongCode && granted && styles.wrong]} accessibilityLiveRegion="polite">
          {permission && !granted && blocked
            ? "Rift may not use the camera. Turn it on in Settings, or paste the pairing link instead."
            : granted && wrongCode
              ? "That QR code is not a Rift pairing code."
              : "Rift › Settings › Remote access › Pair a phone"}
        </Text>
        <View style={styles.flex} />
        {permission && !granted ? (
          blocked ? (
            <Button label="Open Settings" variant="primary" style={styles.wide} onPress={() => void Linking.openSettings()} />
          ) : (
            <Button label="Allow camera" variant="primary" style={styles.wide} onPress={() => void requestPermission()} />
          )
        ) : null}
        <Button label="Paste a pairing link" variant="text" style={styles.wide} onPress={() => router.replace("/add")} />
      </ScrollView>
    </View>
  );
}

const C = 46;
const useStyles = makeStyles((colors) => ({
  fill: { flex: 1, backgroundColor: colors.camera },
  flex: { flex: 1 },
  body: { flexGrow: 1, alignItems: "center", paddingTop: 24, paddingHorizontal: 24, gap: 18 },
  viewfinder: { borderRadius: 16, backgroundColor: colors.viewfinder, overflow: "hidden" },
  corner: { position: "absolute", width: C, height: C, borderColor: colors.accent },
  tl: { left: 22, top: 22, borderLeftWidth: 3, borderTopWidth: 3, borderTopLeftRadius: 10 },
  tr: { right: 22, top: 22, borderRightWidth: 3, borderTopWidth: 3, borderTopRightRadius: 10 },
  bl: { left: 22, bottom: 22, borderLeftWidth: 3, borderBottomWidth: 3, borderBottomLeftRadius: 10 },
  br: { right: 22, bottom: 22, borderRightWidth: 3, borderBottomWidth: 3, borderBottomRightRadius: 10 },
  title: { alignSelf: "stretch", fontFamily: ui(600), color: colors.textBright, fontSize: 20, lineHeight: 26 },
  text: { fontFamily: ui(400), color: colors.textMuted, fontSize: 14, lineHeight: 20, textAlign: "center" },
  // a refusal, said in words at full strength (orange is for "needs you" only)
  wrong: { color: colors.textBright },
  wide: { alignSelf: "stretch" },
}));
