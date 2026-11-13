import { invoke as tauriInvoke } from "@tauri-apps/api/core";

/** The kinds of failure a backend command reports; `ErrorCode` in `src-tauri/src/error.rs`. */
export type ErrorCode =
  | "firstRunSetupRequired"
  | "jobNotFound"
  | "invalidJobState"
  | "invalidRequest"
  | "fileNotFound"
  | "sourceRefused"
  | "storage"
  | "network"
  | "internal";

/** A failed backend command. `message` is the sentence to show; `code` says what kind it is. */
export class CommandError extends Error {
  readonly code: ErrorCode;

  constructor(code: ErrorCode, message: string) {
    super(message);
    this.name = "CommandError";
    this.code = code;
  }
}

/**
 * Calls a backend command. A failure always arrives as an `Error` with a message, whatever the
 * backend sent: its `{ code, message }`, or a bare string from Tauri itself.
 */
export async function invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await tauriInvoke<T>(command, args);
  } catch (error) {
    throw toCommandError(error);
  }
}

export function toCommandError(error: unknown): Error {
  if (error instanceof Error) {
    return error;
  }
  if (isCodedError(error)) {
    return new CommandError(error.code as ErrorCode, error.message);
  }
  return new CommandError("internal", typeof error === "string" ? error : "The command failed.");
}

function isCodedError(value: unknown): value is { code: string; message: string } {
  return (
    typeof value === "object" &&
    value !== null &&
    "code" in value &&
    typeof value.code === "string" &&
    "message" in value &&
    typeof value.message === "string"
  );
}
