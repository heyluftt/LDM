import type { ToolStatus, YtDlpVersionInfo } from "../../shared/api/tools";
import { toolSourceLabel } from "./settingsData";

export function RuntimeToolsSection({ tools, ytDlp }: { tools: ToolStatus[]; ytDlp: YtDlpVersionInfo | null }) {
  return (
    <section className="panel">
      <div className="panel-header">
        <div>
          <h2>Runtime Tools</h2>
        </div>
      </div>
      <div className="tool-status-list">
        {tools.map((tool) => (
          <div className="tool-status-row" key={tool.kind}>
            <div>
              <strong>{tool.name}</strong>
              <span>{tool.required ? "Required" : "Optional"}</span>
            </div>
            <div className={tool.found ? "tool-badge found" : "tool-badge missing"}>
              {tool.found ? "Found" : "Missing"}
            </div>
            <div className="tool-status-detail" title={tool.path ?? tool.message}>
              {tool.found ? `${toolSourceLabel(tool.source)}: ${tool.path}` : tool.message}
            </div>
            {tool.kind === "ytDlp" && tool.found ? <YtDlpVersionLine info={ytDlp} /> : null}
          </div>
        ))}
        {tools.length === 0 ? <div className="empty-state">Tool status unavailable</div> : null}
      </div>
    </section>
  );
}

/**
 * Installed and latest yt-dlp version. yt-dlp breaks whenever a site changes, so an outdated
 * copy is the usual reason provider downloads fail; the update itself stays a manual command.
 */
function YtDlpVersionLine({ info }: { info: YtDlpVersionInfo | null }) {
  if (!info) {
    return <div className="tool-version-line">Checking version…</div>;
  }

  const installed = info.installed
    ? `Installed ${info.installed}`
    : (info.installedError ?? "Installed version unknown");
  const latest = info.latest ? `Latest ${info.latest}` : "Latest version unavailable";

  return (
    <>
      <div className="tool-version-line" title={info.latestError ?? info.installedError ?? undefined}>
        {installed} · {latest}
      </div>
      {info.updateAvailable ? (
        <div className="tool-update-note">
          Update available. Run <code>{info.updateCommand}</code> in a terminal, then restart LDM.
        </div>
      ) : null}
    </>
  );
}
