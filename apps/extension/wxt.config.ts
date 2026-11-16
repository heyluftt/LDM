import { defineConfig } from "wxt";
import { EXTENSION_PUBLIC_KEY } from "./src/shared/extension-identity";
import { manifestVersion } from "./workspace-version.js";

export default defineConfig({
  manifestVersion: 3,
  srcDir: "src",
  // `wxt zip` writes the release archive to .output/.
  zip: {
    name: "ldm-extension",
    artifactTemplate: "{{name}}-{{versionName}}.zip",
  },
  manifest: {
    name: "LDM Extension",
    description: "Detects browser media and sends download jobs to the local desktop app.",
    ...manifestVersion(),
    key: EXTENSION_PUBLIC_KEY,
    permissions: ["downloads", "nativeMessaging", "notifications", "storage", "webRequest"],
    host_permissions: ["<all_urls>"],
    action: {
      default_title: "LDM",
    },
  },
});
