import { Clock3, FolderCog, ListChecks } from "lucide-react";
import { useEffect, useState } from "react";
import { CompletionToast } from "./features/completion/CompletionToast";
import { HistoryView } from "./features/history/HistoryView";
import { FirstRunNotice } from "./features/legal/FirstRunNotice";
import { QueueView } from "./features/queue/QueueView";
import { SettingsView } from "./features/settings/SettingsView";
import { UpdateBanner } from "./features/updates/UpdateBanner";
import { WindowTitlebar } from "./features/window/WindowTitlebar";
import { BrandMark } from "./shared/BrandMark";
import { startJobsSync } from "./shared/jobsStore";
import { startUpdateChecks } from "./shared/updateStore";

const tabs = [
  { id: "queue", label: "Queue", icon: ListChecks },
  { id: "history", label: "History", icon: Clock3 },
  { id: "settings", label: "Settings", icon: FolderCog },
] as const;

export function App() {
  const [activeTab, setActiveTab] = useState<(typeof tabs)[number]["id"]>("queue");
  const activeTabLabel = tabs.find((tab) => tab.id === activeTab)?.label ?? "Queue";

  useEffect(() => startJobsSync(), []);
  useEffect(() => startUpdateChecks(), []);

  return (
    <main className={`app-window app-window-${activeTab}`}>
      <WindowTitlebar section={activeTabLabel} />
      <div className="app-shell">
        <header className="compact-header">
          <div className="brand compact-brand">
            <span className="brand-mark" aria-hidden="true">
              <BrandMark size={28} />
            </span>
            <span className="brand-copy">
              <strong>LDM</strong>
              <small>Local downloads</small>
            </span>
          </div>
          <nav className="nav compact-tabs" aria-label="LDM views">
            {tabs.map((tab) => {
              const Icon = tab.icon;
              return (
                <button
                  className={tab.id === activeTab ? "nav-item active" : "nav-item"}
                  key={tab.id}
                  onClick={() => setActiveTab(tab.id)}
                  type="button"
                  title={tab.label}
                >
                  <Icon size={15} />
                  <span>{tab.label}</span>
                </button>
              );
            })}
          </nav>
        </header>
        <section className="workspace">
          <UpdateBanner />
          {activeTab === "queue" ? <QueueView /> : null}
          {activeTab === "history" ? <HistoryView /> : null}
          {activeTab === "settings" ? <SettingsView /> : null}
        </section>
      </div>
      <CompletionToast />
      <FirstRunNotice />
    </main>
  );
}
