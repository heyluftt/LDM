import type { ExtensionContact } from "@ldm/contracts";
import { invoke } from "./invoke";

export function getExtensionContact(): Promise<ExtensionContact | null> {
  return invoke<ExtensionContact | null>("extension_contact");
}
