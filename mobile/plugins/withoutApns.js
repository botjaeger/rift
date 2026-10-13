// A free Apple ID ("personal team") cannot sign an app with the Push Notifications capability,
// and expo-notifications adds it (`aps-environment` in the entitlements) on every prebuild. With
// RIFT_NO_APNS=1 this plugin takes it out again, so the build signs; Expo push alerts are then
// unavailable, and alerts while the app is closed come through ntfy (README › Push).
const { withEntitlementsPlist } = require("expo/config-plugins");

module.exports = function withoutApns(config) {
  if (process.env.RIFT_NO_APNS !== "1") return config;
  return withEntitlementsPlist(config, (c) => {
    delete c.modResults["aps-environment"];
    return c;
  });
};
