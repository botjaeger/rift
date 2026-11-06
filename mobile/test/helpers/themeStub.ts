// What a test that runs a component in a bare context gets for `ui/theme`: the palette hooks
// over Dark Default's palette (palette.ts is free of React Native), the faces as plain names.
import { DARK_DEFAULT, paletteFrom, type Palette } from "../../src/ui/palette";

export const stubColors: Palette = paletteFrom(DARK_DEFAULT);

export const themeStub = {
  makeStyles: (factory: (c: Palette) => object) => () => factory(stubColors),
  useColors: () => stubColors,
  fromPalette:
    <T>(make: (c: Palette) => T) =>
    (c: Palette) =>
      make(c),
  PaletteProvider: "PaletteProvider",
  paletteOf: () => stubColors,
  tint: (hex: string, alpha: number) => `${hex.slice(0, 7)}${alpha.toString(16).padStart(2, "0")}`,
  ui: () => "ui",
  mono: "mono",
  monoMedium: "monoMedium",
  monoBold: "monoBold",
  radius: { control: 6, small: 6, button: 10, field: 10, chip: 16, card: 14, sheet: 20 },
  TOUCH: 44,
};
