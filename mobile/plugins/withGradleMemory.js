// The prebuild template gives Gradle 2 GB of heap and 512 MB of metaspace, and a release build
// (`gradlew assembleRelease`) runs out of metaspace in react-native-screens' release lint. Raise
// both so release builds, which carry expo-updates for OTA updates, finish.
const { withGradleProperties } = require("expo/config-plugins");

const JVM_ARGS = "-Xmx4096m -XX:MaxMetaspaceSize=1024m";

module.exports = function withGradleMemory(config) {
  return withGradleProperties(config, (c) => {
    const props = c.modResults.filter(
      (p) => !(p.type === "property" && p.key === "org.gradle.jvmargs"),
    );
    props.push({ type: "property", key: "org.gradle.jvmargs", value: JVM_ARGS });
    c.modResults = props;
    return c;
  });
};
