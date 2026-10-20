// The Expo account that publishes Rift's updates stays out of the repo: its owner
// and EAS project id come from the environment (EXPO_OWNER, EAS_PROJECT_ID) or
// from ~/.config/rift-release/expo.env (KEY=value lines). With them a build gets
// over-the-air updates and Expo push; without them (a contributor's build) it
// gets neither, and the fingerprint differs from the released builds'.
const fs = require("fs");
const os = require("os");
const path = require("path");

function account() {
  const env = { EXPO_OWNER: process.env.EXPO_OWNER, EAS_PROJECT_ID: process.env.EAS_PROJECT_ID };
  const file = process.env.RIFT_EXPO_ENV_FILE || path.join(os.homedir(), ".config/rift-release/expo.env");
  let text = "";
  try {
    text = fs.readFileSync(file, "utf8");
  } catch {
    // no file: the environment alone decides
  }
  for (const line of text.split("\n")) {
    const m = line.match(/^\s*(EXPO_OWNER|EAS_PROJECT_ID)\s*=\s*(\S+)\s*$/);
    if (m && !env[m[1]]) env[m[1]] = m[2];
  }
  return env;
}

module.exports = ({ config }) => {
  const { EXPO_OWNER: owner, EAS_PROJECT_ID: projectId } = account();
  if (!owner || !projectId) return config;
  return {
    ...config,
    owner,
    updates: { ...config.updates, url: `https://u.expo.dev/${projectId}` },
    extra: { ...config.extra, eas: { ...config.extra?.eas, projectId } },
  };
};
