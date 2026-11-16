import { Minus, Square, X } from "lucide-react";
import type { MouseEvent } from "react";
import { BrandMark } from "../../shared/BrandMark";
import {
  closeAppWindow,
  minimizeAppWindow,
  startAppWindowDrag,
  toggleAppWindowMaximize,
} from "../../shared/windowControls";

export function WindowTitlebar({ section }: { section: string }) {
  const startDrag = (event: MouseEvent<HTMLElement>) => {
    if (event.button !== 0 || event.detail > 1 || isWindowControl(event.target)) {
      return;
    }

    void startAppWindowDrag();
  };

  const toggleMaximize = (event: MouseEvent<HTMLElement>) => {
    if (isWindowControl(event.target)) {
      return;
    }

    void toggleAppWindowMaximize();
  };

  return (
    // biome-ignore lint/a11y/noStaticElementInteractions: the titlebar is the window's mouse drag region; keyboard users have the window control buttons
    <header className="app-window-bar" onMouseDown={startDrag} onDoubleClick={toggleMaximize}>
      <div className="app-window-title">
        <span className="app-window-icon" aria-hidden="true">
          <BrandMark size={17} />
        </span>
        <strong>LDM</strong>
      </div>
      <span className="app-window-section">{section}</span>
      <div className="window-controls" role="toolbar" aria-label="Window controls">
        <button type="button" title="Minimize" onClick={() => void minimizeAppWindow()}>
          <Minus size={14} strokeWidth={2.4} />
        </button>
        <button type="button" title="Maximize" onClick={() => void toggleAppWindowMaximize()}>
          <Square size={12} strokeWidth={2.2} />
        </button>
        <button className="window-control-close" type="button" title="Close" onClick={() => void closeAppWindow()}>
          <X size={15} strokeWidth={2.3} />
        </button>
      </div>
    </header>
  );
}

function isWindowControl(target: EventTarget): boolean {
  return target instanceof Element && target.closest(".window-controls") !== null;
}
