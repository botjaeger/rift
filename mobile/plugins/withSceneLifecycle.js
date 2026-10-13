// iOS 27 stops an app at launch unless it adopts the UIScene life cycle. Expo SDK 57 ships the
// scene delegate (ExpoAppSceneDelegate) but its app template does not use it yet, so this plugin
// wires it in on every prebuild (same plugin as Harbor's): the scene manifest in Info.plist, and an AppDelegate that
// hands its React Native factory to the scene delegate instead of opening its own window.
const { withAppDelegate, withInfoPlist } = require("expo/config-plugins");

const WINDOW_START = `#if os(iOS) || os(tvOS)
    window = UIWindow(frame: UIScreen.main.bounds)
    factory.startReactNative(
      withModuleName: "main",
      in: window,
      launchOptions: launchOptions)
#endif
`;

module.exports = function withSceneLifecycle(config) {
  config = withInfoPlist(config, (c) => {
    c.modResults.UIApplicationSceneManifest = {
      UIApplicationSupportsMultipleScenes: false,
      UISceneConfigurations: {
        UIWindowSceneSessionRoleApplication: [
          {
            UISceneConfigurationName: "Default Configuration",
            UISceneDelegateClassName: "EXExpoAppSceneDelegate",
          },
        ],
      },
    };
    return c;
  });
  return withAppDelegate(config, (c) => {
    let src = c.modResults.contents;
    if (!src.includes("ExpoReactNativeFactoryProvider")) {
      src = src.replace(
        "class AppDelegate: ExpoAppDelegate {",
        "class AppDelegate: ExpoAppDelegate, ExpoReactNativeFactoryProvider {",
      );
    }
    if (src.includes(WINDOW_START)) {
      src = src.replace(
        WINDOW_START,
        "    // The scene delegate opens the window and starts React Native into it.\n",
      );
    }
    if (!src.includes("ExpoReactNativeFactoryProvider") || src.includes("UIWindow(frame:")) {
      throw new Error("withSceneLifecycle: the AppDelegate template changed; update the plugin.");
    }
    c.modResults.contents = src;
    return c;
  });
};
