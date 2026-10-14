// The lock screen's Live Activity for a running command (the P6 Lock board): a WidgetKit
// extension "RiftLive" (targets/RiftLive), embedded in the app, and NSSupportsLiveActivities.
// The extension needs no entitlement, so a free Apple ID ("personal team") signs it like the
// app. The app starts and ends activities itself (modules/rift-live); updating or ending one
// while the app is suspended would need ActivityKit push (APNs), which such a build lacks.
const fs = require("fs");
const path = require("path");
const { withDangerousMod, withInfoPlist, withXcodeProject } = require("expo/config-plugins");

const NAME = "RiftLive";
const SOURCES = ["RiftLiveWidget.swift", "RiftLiveAttributes.swift"];

function withFiles(config) {
  return withDangerousMod(config, [
    "ios",
    async (c) => {
      const root = c.modRequest.projectRoot;
      const from = path.join(root, "targets", NAME);
      // the app's copy of the attributes must match the extension's (ActivityKit pairs them)
      const app = fs.readFileSync(path.join(root, "modules", "rift-live", "ios", "RiftLiveAttributes.swift"), "utf8");
      if (app !== fs.readFileSync(path.join(from, "RiftLiveAttributes.swift"), "utf8"))
        throw new Error("modules/rift-live/ios/RiftLiveAttributes.swift differs from targets/RiftLive/RiftLiveAttributes.swift");
      const to = path.join(c.modRequest.platformProjectRoot, NAME);
      fs.mkdirSync(to, { recursive: true });
      for (const f of [...SOURCES, "Info.plist"]) fs.copyFileSync(path.join(from, f), path.join(to, f));
      return c;
    },
  ]);
}

function withTarget(config) {
  return withXcodeProject(config, (c) => {
    const proj = c.modResults;
    if (proj.pbxTargetByName(NAME)) return c;
    const bundleId = `${c.ios.bundleIdentifier}.live`;
    const target = proj.addTarget(NAME, "app_extension", NAME, bundleId);
    const group = proj.addPbxGroup([...SOURCES, "Info.plist"], NAME, NAME);
    const main = proj.getFirstProject().firstProject.mainGroup;
    proj.addToPbxGroup(group.uuid, main);
    proj.addBuildPhase(SOURCES, "PBXSourcesBuildPhase", "Sources", target.uuid, undefined, NAME);
    proj.addBuildPhase([], "PBXResourcesBuildPhase", "Resources", target.uuid);
    proj.addBuildPhase([], "PBXFrameworksBuildPhase", "Frameworks", target.uuid);

    // the app target's team and version, so both sign and ship together
    const configs = proj.pbxXCBuildConfigurationSection();
    let team;
    let version;
    for (const k of Object.keys(configs)) {
      const s = configs[k].buildSettings;
      if (s && s.PRODUCT_BUNDLE_IDENTIFIER && String(s.PRODUCT_BUNDLE_IDENTIFIER).replace(/"/g, "") === c.ios.bundleIdentifier) {
        team = team || s.DEVELOPMENT_TEAM;
        version = version || s.MARKETING_VERSION;
      }
    }
    for (const k of Object.keys(configs)) {
      const s = configs[k].buildSettings;
      if (!s || String(s.PRODUCT_NAME).replace(/"/g, "") !== NAME) continue;
      Object.assign(s, {
        INFOPLIST_FILE: `${NAME}/Info.plist`,
        PRODUCT_BUNDLE_IDENTIFIER: bundleId,
        SWIFT_VERSION: "5.0",
        IPHONEOS_DEPLOYMENT_TARGET: "16.4",
        TARGETED_DEVICE_FAMILY: '"1,2"',
        CODE_SIGN_STYLE: "Automatic",
        GENERATE_INFOPLIST_FILE: "NO",
        SKIP_INSTALL: "YES",
        CURRENT_PROJECT_VERSION: "1",
        MARKETING_VERSION: version || "1.0",
        LD_RUNPATH_SEARCH_PATHS: '"$(inherited) @executable_path/Frameworks @executable_path/../../Frameworks"',
        ...(team ? { DEVELOPMENT_TEAM: team } : {}),
      });
    }
    return c;
  });
}

module.exports = function withLiveActivity(config) {
  config = withInfoPlist(config, (c) => {
    c.modResults.NSSupportsLiveActivities = true;
    return c;
  });
  return withTarget(withFiles(config));
};
