import { defineConfig } from "vitest/config";

// Only the app's pure logic runs here (protocol, client state machine, store); screens are
// checked on the web target, simulators and devices.
export default defineConfig({
  test: {
    environment: "node",
    include: ["test/**/*.test.ts"],
  },
});
