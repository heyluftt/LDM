import type { ExtensionContact } from "@ldm/contracts";
import { Circle, CircleCheck, Clipboard } from "lucide-react";
import { type ReactNode, useEffect, useState } from "react";
import { getExtensionContact } from "../../shared/api/setup";
import { getToolStatus, type ToolStatus } from "../../shared/api/tools";
import { isSetupComplete, type SetupStep, setupSteps } from "./gettingStartedModel";

/** While something is missing, the panel looks again this often, so a step turns green on its own. */
const RECHECK_INTERVAL_MS = 5000;

/**
 * What a new installation still needs: the browser extension, yt-dlp, FFmpeg. It takes the
 * place of the empty download list until all three are there or the user hides it; `fallback`
 * is what the list shows otherwise.
 */
export function GettingStarted({ fallback }: { fallback: ReactNode }) {
  const [setup, setSetup] = useState<{ contact: ExtensionContact | null; tools: ToolStatus[] } | null>(null);
  const [hidden, setHidden] = useState(hasHiddenGettingStarted);
  const [copiedCommand, setCopiedCommand] = useState<string | null>(null);

  const steps = setup ? setupSteps(setup.contact, setup.tools) : null;
  const complete = steps ? isSetupComplete(steps) : false;
  const watching = !hidden && !complete;

  useEffect(() => {
    if (!watching) {
      return;
    }
    let active = true;
    const check = () => {
      void Promise.all([getExtensionContact(), getToolStatus()])
        .then(([contact, report]) => {
          if (active) {
            setSetup({ contact, tools: report.tools });
          }
        })
        .catch(() => undefined);
    };
    check();
    const timer = window.setInterval(check, RECHECK_INTERVAL_MS);
    return () => {
      active = false;
      window.clearInterval(timer);
    };
  }, [watching]);

  if (hidden || !steps || complete) {
    return fallback;
  }

  const extensionConnected = steps.some((step) => step.id === "extension" && step.done);
  return (
    <section className="getting-started" aria-labelledby="getting-started-title">
      <div className="getting-started-header">
        <div>
          <strong id="getting-started-title">Getting started</strong>
          <span>
            {extensionConnected
              ? "Plain file downloads work. yt-dlp and FFmpeg add YouTube, X, TikTok, and streams."
              : "Plain file downloads work as soon as the extension is connected."}
          </span>
        </div>
        <button
          className="secondary-action"
          type="button"
          onClick={() => {
            rememberGettingStartedHidden();
            setHidden(true);
          }}
        >
          Hide
        </button>
      </div>
      <ol className="getting-started-steps">
        {steps.map((step) => (
          <SetupStepRow
            key={step.id}
            step={step}
            copied={copiedCommand === step.command}
            onCopy={(command) => {
              void navigator.clipboard.writeText(command);
              setCopiedCommand(command);
            }}
          />
        ))}
      </ol>
    </section>
  );
}

function SetupStepRow({
  step,
  copied,
  onCopy,
}: {
  step: SetupStep;
  copied: boolean;
  onCopy: (command: string) => void;
}) {
  if (step.done) {
    // One line: what is done needs no explanation.
    return (
      <li className="setup-step done">
        <CircleCheck className="setup-step-icon" size={16} aria-label="Done" />
        <div className="setup-step-copy">
          <strong>
            {step.title} <span>{step.status}</span>
          </strong>
        </div>
      </li>
    );
  }

  const command = step.command;
  return (
    <li className="setup-step">
      <Circle className="setup-step-icon" size={16} aria-label="Open" />
      <div className="setup-step-copy">
        <strong>{step.title}</strong>
        <span>{step.status}</span>
        {step.hint ? <small>{step.hint}</small> : null}
        {command ? (
          <div className="setup-step-command">
            <code>{command}</code>
            <button className="diagnostic-button" type="button" onClick={() => onCopy(command)}>
              <Clipboard size={13} />
              <span>{copied ? "Copied" : "Copy"}</span>
            </button>
          </div>
        ) : null}
      </div>
    </li>
  );
}

const GETTING_STARTED_HIDDEN_KEY = "ldm.gettingStartedHidden";

function hasHiddenGettingStarted(): boolean {
  return window.localStorage.getItem(GETTING_STARTED_HIDDEN_KEY) === "1";
}

function rememberGettingStartedHidden(): void {
  window.localStorage.setItem(GETTING_STARTED_HIDDEN_KEY, "1");
}
