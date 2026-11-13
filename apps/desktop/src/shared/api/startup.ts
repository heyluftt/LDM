import { invoke } from "./invoke";

export type StartupStatus = {
  available: boolean;
  enabled: boolean;
  reason?: string | null;
};

export function getStartupStatus(): Promise<StartupStatus> {
  return invoke<StartupStatus>("startup_status");
}

export function setStartupEnabled(enabled: boolean): Promise<StartupStatus> {
  return invoke<StartupStatus>("set_startup_enabled", { enabled });
}
