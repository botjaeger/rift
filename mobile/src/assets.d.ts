// An image the bundler packs (Metro picks the @2x / @3x file for the screen): its source.
declare module "*.png" {
  import type { ImageSourcePropType } from "react-native";
  const source: ImageSourcePropType;
  export default source;
}
