import type { DesktopToExtensionMessage, ExtensionToDesktopMessage } from "@ldm/contracts";

import { prewarmNativeMessaging, sendQueuedNativeMessage } from "./native-messaging-client";

export { prewarmNativeMessaging };

export async function sendNativeMessage(message: ExtensionToDesktopMessage): Promise<DesktopToExtensionMessage> {
  return sendQueuedNativeMessage(message);
}
