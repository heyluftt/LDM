import type { DesktopToExtensionMessage, ExtensionToDesktopMessage } from "@ldm/contracts";

const HOST_NAME = "dev.ldm.native_host";
/** Normal reply budget: the host answers from the running desktop app within a second. */
const RESPONSE_TIMEOUT_MS = 30_000;
/** Budget once the host reports that it is launching the desktop app first. */
const DESKTOP_STARTING_TIMEOUT_MS = 90_000;
const IDLE_DISCONNECT_MS = 5 * 60_000;

type PendingNativeMessage = {
  message: ExtensionToDesktopMessage;
  resolve: (response: DesktopToExtensionMessage) => void;
  timeoutId?: ReturnType<typeof setTimeout>;
};

let nativePort: chrome.runtime.Port | null = null;
let activeMessage: PendingNativeMessage | null = null;
const queuedMessages: PendingNativeMessage[] = [];
let idleDisconnectTimer: ReturnType<typeof setTimeout> | undefined;

export function prewarmNativeMessaging(): void {
  try {
    ensureNativePort();
    scheduleIdleDisconnect();
  } catch {
    resetNativePort();
  }
}

export async function sendQueuedNativeMessage(message: ExtensionToDesktopMessage): Promise<DesktopToExtensionMessage> {
  return new Promise((resolve) => {
    queuedMessages.push({ message, resolve });
    pumpNativeQueue();
  });
}

function pumpNativeQueue(): void {
  if (activeMessage || queuedMessages.length === 0) {
    return;
  }

  const nextMessage = queuedMessages.shift();
  if (!nextMessage) {
    return;
  }

  let port: chrome.runtime.Port;
  try {
    port = ensureNativePort();
  } catch {
    nextMessage.resolve(desktopUnavailableResponse());
    globalThis.setTimeout(pumpNativeQueue, 0);
    return;
  }

  clearIdleDisconnectTimer();
  activeMessage = nextMessage;
  armActiveMessageTimeout(RESPONSE_TIMEOUT_MS);

  try {
    port.postMessage(nextMessage.message);
  } catch {
    completeActiveMessage(desktopUnavailableResponse());
    resetNativePort();
  }
}

function armActiveMessageTimeout(timeoutMs: number): void {
  const pending = activeMessage;
  if (!pending) {
    return;
  }
  if (pending.timeoutId !== undefined) {
    globalThis.clearTimeout(pending.timeoutId);
  }
  pending.timeoutId = globalThis.setTimeout(() => {
    completeActiveMessage(desktopUnavailableResponse("Desktop app took too long to respond."));
    resetNativePort();
  }, timeoutMs);
}

function ensureNativePort(): chrome.runtime.Port {
  if (nativePort) {
    return nativePort;
  }

  const port = chrome.runtime.connectNative(HOST_NAME);
  port.onMessage.addListener(handleNativeMessage);
  port.onDisconnect.addListener(handleNativeDisconnect);
  nativePort = port;
  return port;
}

function handleNativeMessage(response: DesktopToExtensionMessage | undefined): void {
  if (response?.type === "desktopStarting") {
    // The host is launching the desktop app and will answer once it is up; give it time.
    armActiveMessageTimeout(DESKTOP_STARTING_TIMEOUT_MS);
    return;
  }

  completeActiveMessage(response ?? desktopUnavailableResponse());
}

function handleNativeDisconnect(): void {
  const reason = chrome.runtime.lastError?.message;
  resetNativePort();

  if (activeMessage) {
    completeActiveMessage(desktopUnavailableResponse(reason));
  }

  if (queuedMessages.length > 0) {
    globalThis.setTimeout(pumpNativeQueue, 0);
  }
}

function completeActiveMessage(response: DesktopToExtensionMessage): void {
  const pending = activeMessage;
  if (!pending) {
    return;
  }

  if (pending.timeoutId !== undefined) {
    globalThis.clearTimeout(pending.timeoutId);
  }
  activeMessage = null;
  pending.resolve(response);
  scheduleIdleDisconnect();
  globalThis.setTimeout(pumpNativeQueue, 0);
}

function scheduleIdleDisconnect(): void {
  clearIdleDisconnectTimer();
  idleDisconnectTimer = globalThis.setTimeout(() => {
    if (!activeMessage && queuedMessages.length === 0) {
      resetNativePort();
    }
  }, IDLE_DISCONNECT_MS);
}

function clearIdleDisconnectTimer(): void {
  if (idleDisconnectTimer !== undefined) {
    globalThis.clearTimeout(idleDisconnectTimer);
    idleDisconnectTimer = undefined;
  }
}

function resetNativePort(): void {
  clearIdleDisconnectTimer();
  if (nativePort) {
    nativePort.onMessage.removeListener(handleNativeMessage);
    nativePort.onDisconnect.removeListener(handleNativeDisconnect);
    try {
      nativePort.disconnect();
    } catch {
      // The browser already closed this port.
    }
  }
  nativePort = null;
}

function desktopUnavailableResponse(reason?: string): DesktopToExtensionMessage {
  return {
    type: "jobRejected",
    schemaVersion: 1,
    reason: reason ? `Desktop app could not be reached. ${reason}` : "Desktop app could not be reached.",
    code: "desktopUnavailable",
  };
}
