import { Platform, useWindowDimensions } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";

/**
 * Each sheet's height on the Sheets boards (402 × 874 pt), from its top edge to the screen's foot.
 * The sheet is sized to its content (`fitToContents`), and its content is this tall, so the
 * actions pinned to its foot stay there whatever the content above them holds.
 */
export const SHEET_HEIGHTS = {
  attach: 484,
  folder: 645,
  block: 690,
  newTab: 645,
  newPane: 507,
  confirm: 381,
  whatsNew: 484,
  connection: 541,
  localPage: 484,
} as const;

export type SheetBoard = keyof typeof SHEET_HEIGHTS;

/** Room kept above a sheet on a short screen: it never reaches the status bar. */
export const SHEET_TOP_ROOM = 24;

/**
 * The content's height for the board's sheet, or less on a screen too short for it (`height`:
 * the window's; `top`: its safe-area inset). `under`: what the system adds under a sheet's
 * content (iOS: the home indicator's inset), so the sheet itself is the board's height.
 */
export function sheetHeight(board: SheetBoard, height: number, top: number, under = 0): number {
  return Math.round(Math.max(0, Math.min(SHEET_HEIGHTS[board], height - top - SHEET_TOP_ROOM) - under));
}

/**
 * What a platform adds under a fitToContents sheet's content: iOS 27 sizes the sheet to the
 * content plus the bottom safe-area inset (measured on the iPhone 18 Pro sim: 541 pt of content,
 * a sheet 34 pt taller); Android's sheet is the content alone.
 */
export const sheetUnder = (os: string, bottom: number) => (os === "ios" ? bottom : 0);

/** A sheet's content height on this screen; undefined on the web, where a sheet is a plain page. */
export function useSheetHeight(board: SheetBoard): number | undefined {
  const { height } = useWindowDimensions();
  const insets = useSafeAreaInsets();
  return Platform.OS === "web" ? undefined : sheetHeight(board, height, insets.top, sheetUnder(Platform.OS, insets.bottom));
}
