import type { ExtensionContact } from "@ldm/contracts";
import { invoke } from "./invoke";

/** When a browser last started the native host for the extension; `null` when it never did. */
export function getExtensionContact(): Promise<ExtensionContact | null> {
  return invoke<ExtensionContact | null>("extension_contact");
}
