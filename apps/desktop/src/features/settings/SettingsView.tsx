import type { DownloadKind } from "@ldm/contracts";
import { useEffect, useState } from "react";
import { getLegalStatus, type LegalStatus } from "../../shared/api/legal";
import {
  type ConcurrencySettings,
  getConcurrencySettings,
  getTargetFolders,
  pickTargetFolder,
  setMaxConcurrentDownloads,
  setTargetFolder,
  type TargetFolder,
} from "../../shared/api/settings";
import { getStartupStatus, type StartupStatus, setStartupEnabled } from "../../shared/api/startup";
import { getToolStatus, loadYtDlpVersionInfo, type ToolStatus, type YtDlpVersionInfo } from "../../shared/api/tools";
import {
  type AppColorMode,
  type AppTheme,
  getAppColorMode,
  getAppTheme,
  setAppColorMode,
  setAppTheme,
} from "../../shared/appearancePreferences";
import {
  type BrowserDownloadMode,
  type BrowserIntegrationPreferences,
  defaultBrowserIntegrationPreferences,
  getBrowserIntegrationPreferences,
  getNativeHostRegistration,
  type NativeHostRegistration,
  setBrowserIntegrationPreferences,
} from "../../shared/browserIntegrationPreferences";
import {
  type CompletionPreferences,
  getCompletionPreferences,
  setCompletionPreferences,
} from "../../shared/completionPreferences";
import { type LegalDocumentId, legalDocumentById } from "../../shared/legal";
import { AppearanceSection } from "./AppearanceSection";
import { BrowserIntegrationSection } from "./BrowserIntegrationSection";
import { CompletionSettingsSection } from "./CompletionSettingsSection";
import { FolderSettingsSection } from "./FolderSettingsSection";
import { LegalDocumentDialog } from "./LegalDocumentDialog";
import { LegalSettingsSection } from "./LegalSettingsSection";
import { ParallelDownloadsSection } from "./ParallelDownloadsSection";
import { RuntimeToolsSection } from "./RuntimeToolsSection";
import { StartupSection } from "./StartupSection";
import { normalizeDomainRuleHost } from "./settingsData";
import { UpdatesSection } from "./UpdatesSection";

export function SettingsView() {
  const [tools, setTools] = useState<ToolStatus[]>([]);
  const [ytDlp, setYtDlp] = useState<YtDlpVersionInfo | null>(null);
  const [theme, setTheme] = useState<AppTheme>(getAppTheme);
  const [colorMode, setColorMode] = useState<AppColorMode>(getAppColorMode);
  const [completionPreferences, setLocalCompletionPreferences] = useState(getCompletionPreferences);
  const [legalStatus, setLegalStatus] = useState<LegalStatus | null>(null);
  const [startupStatus, setStartupStatus] = useState<StartupStatus | null>(null);
  const [startupError, setStartupError] = useState<string | null>(null);
  const [legalDocumentId, setLegalDocumentId] = useState<LegalDocumentId | null>(null);
  const [browserPreferences, setLocalBrowserPreferences] = useState(defaultBrowserIntegrationPreferences);
  const [browserPreferencesError, setBrowserPreferencesError] = useState<string | null>(null);
  const [nativeHostRegistration, setNativeHostRegistration] = useState<NativeHostRegistration | null>(null);
  const [domainHost, setDomainHost] = useState("");
  const [domainAction, setDomainAction] = useState<BrowserDownloadMode>("ask");
  const [targetFolders, setTargetFolders] = useState<TargetFolder[]>([]);
  const [folderError, setFolderError] = useState<string | null>(null);
  const [busyFolderKind, setBusyFolderKind] = useState<DownloadKind | null>(null);
  const [concurrency, setConcurrency] = useState<ConcurrencySettings | null>(null);
  const [concurrencyError, setConcurrencyError] = useState<string | null>(null);

  // Loaded once when the view opens; everything shown here changes only through this view.
  useEffect(() => {
    let active = true;

    const load = async () => {
      const [
        report,
        nextLegalStatus,
        nextBrowserPreferences,
        nextStartupStatus,
        nextTargetFolders,
        nextConcurrency,
        nextNativeHostRegistration,
      ] = await Promise.all([
        getToolStatus(),
        getLegalStatus(),
        getBrowserIntegrationPreferences(),
        getStartupStatus(),
        getTargetFolders(),
        getConcurrencySettings(),
        getNativeHostRegistration(),
      ]);
      if (active) {
        setTools(report.tools);
        setLegalStatus(nextLegalStatus);
        setLocalBrowserPreferences(nextBrowserPreferences);
        setStartupStatus(nextStartupStatus);
        setTargetFolders(nextTargetFolders);
        setConcurrency(nextConcurrency);
        setNativeHostRegistration(nextNativeHostRegistration);
      }
    };

    void load();

    return () => {
      active = false;
    };
  }, []);

  // Separate from the main load: `yt-dlp --version` and the GitHub lookup may take seconds and
  // must not hold back the rest of the view.
  useEffect(() => {
    let active = true;

    void loadYtDlpVersionInfo()
      .then((info) => {
        if (active) {
          setYtDlp(info);
        }
      })
      .catch(() => {
        // The tool row already shows the discovery result; nothing more to say here.
      });

    return () => {
      active = false;
    };
  }, []);

  const updateCompletionPreferences = (next: CompletionPreferences) => {
    setLocalCompletionPreferences(next);
    setCompletionPreferences(next);
  };

  const updateTheme = (nextTheme: AppTheme) => {
    setTheme(nextTheme);
    setAppTheme(nextTheme);
  };

  const updateColorMode = (nextColorMode: AppColorMode) => {
    setColorMode(nextColorMode);
    setAppColorMode(nextColorMode);
  };

  const updateBrowserPreferences = (next: BrowserIntegrationPreferences) => {
    const extensionIdChanged = next.extensionId !== browserPreferences.extensionId;
    setLocalBrowserPreferences(next);
    setBrowserPreferencesError(null);
    void setBrowserIntegrationPreferences(next)
      .then((saved) => {
        setLocalBrowserPreferences(saved);
        // A new id re-registers the native host; show what came of it.
        return extensionIdChanged ? getNativeHostRegistration().then(setNativeHostRegistration) : undefined;
      })
      .catch(() => {
        setBrowserPreferencesError("Could not save browser integration settings.");
      });
  };

  const updateStartup = (enabled: boolean) => {
    setStartupError(null);
    void setStartupEnabled(enabled)
      .then(setStartupStatus)
      .catch((error: unknown) => {
        setStartupError(error instanceof Error ? error.message : "Could not update startup setting.");
      });
  };

  const updateMaxConcurrentDownloads = (value: number) => {
    setConcurrencyError(null);
    void setMaxConcurrentDownloads(value)
      .then(setConcurrency)
      .catch((error: unknown) => {
        setConcurrencyError(error instanceof Error ? error.message : String(error));
      });
  };

  const runFolderAction = (kind: DownloadKind, action: () => Promise<TargetFolder[]>) => {
    setBusyFolderKind(kind);
    setFolderError(null);
    void action()
      .then(setTargetFolders)
      .catch((error: unknown) => {
        setFolderError(error instanceof Error ? error.message : String(error));
      })
      .finally(() => setBusyFolderKind(null));
  };

  const addDomainRule = () => {
    const host = normalizeDomainRuleHost(domainHost);
    if (!host) {
      return;
    }
    updateBrowserPreferences({
      ...browserPreferences,
      domainRules: [
        ...browserPreferences.domainRules.filter((rule) => rule.host !== host),
        { host, action: domainAction },
      ],
    });
    setDomainHost("");
    setDomainAction("ask");
  };

  const activeLegalDocument = legalDocumentId ? legalDocumentById(legalDocumentId) : null;

  return (
    <>
      <FolderSettingsSection
        busyKind={busyFolderKind}
        error={folderError}
        folders={targetFolders}
        onEdit={(kind, path) => runFolderAction(kind, () => setTargetFolder(kind, path))}
        onPick={(kind) => runFolderAction(kind, () => pickTargetFolder(kind))}
        onReset={(kind) => runFolderAction(kind, () => setTargetFolder(kind, null))}
      />
      <ParallelDownloadsSection
        error={concurrencyError}
        settings={concurrency}
        onChange={updateMaxConcurrentDownloads}
      />
      <StartupSection error={startupError} status={startupStatus} onToggle={updateStartup} />
      <BrowserIntegrationSection
        domainAction={domainAction}
        domainHost={domainHost}
        error={browserPreferencesError}
        nativeHostRegistration={nativeHostRegistration}
        preferences={browserPreferences}
        onAddDomainRule={addDomainRule}
        onChange={updateBrowserPreferences}
        onDomainActionChange={setDomainAction}
        onDomainHostChange={setDomainHost}
      />
      <AppearanceSection
        colorMode={colorMode}
        theme={theme}
        onColorModeChange={updateColorMode}
        onThemeChange={updateTheme}
      />
      <LegalSettingsSection legalStatus={legalStatus} onSelectDocument={setLegalDocumentId} />
      <CompletionSettingsSection preferences={completionPreferences} onChange={updateCompletionPreferences} />
      <RuntimeToolsSection tools={tools} ytDlp={ytDlp} />
      <UpdatesSection />
      <LegalDocumentDialog document={activeLegalDocument} onClose={() => setLegalDocumentId(null)} />
    </>
  );
}
