import { useCallback, useEffect, useRef, useState } from "react";
import { api, errorText } from "./api";
import { Launchpad } from "./components/Launchpad";
import { RepoView } from "./components/RepoView";
import { Resizer, clamp } from "./components/Resizer";
import { SettingsPopover } from "./components/SettingsPopover";
import { SmartFeaturesPanel } from "./components/SmartFeaturesPanel";
import { TabBar, type Tab } from "./components/TabBar";
import { showToast, Toaster } from "./toast";
import type { RepoInfo, Settings, SmartFeaturesUi } from "./types";

const DEFAULT_SETTINGS: Settings = { recent: [], openTabs: [], activeTab: null, autoFetchSeconds: 60 };
const DEFAULT_SF_UI: SmartFeaturesUi = { panelOpen: false, width: 300, collapsed: [], changedOnly: false };
const MAX_RECENT = 30;

let tabCounter = 0;
const newId = () => `tab-${++tabCounter}`;
const launchpadTab = (): Tab => ({ id: newId(), kind: "launchpad" });
const repoTab = (info: RepoInfo): Tab => ({ id: newId(), kind: "repo", path: info.root, name: info.name });

function normalize(raw: Partial<Settings> | null): Settings {
  return {
    ...DEFAULT_SETTINGS,
    ...(raw ?? {}),
    recent: Array.isArray(raw?.recent) ? raw.recent : [],
    openTabs: Array.isArray(raw?.openTabs) ? raw.openTabs : [],
    autoFetchSeconds: typeof raw?.autoFetchSeconds === "number" ? raw.autoFetchSeconds : 60,
    smartFeatures: { ...DEFAULT_SF_UI, ...(raw?.smartFeatures ?? {}) },
  };
}

export default function App() {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [tabs, setTabs] = useState<Tab[]>([]);
  const [activeId, setActiveId] = useState("");
  const [showSettings, setShowSettings] = useState(false);
  const [cloneDir, setCloneDir] = useState("");
  const tabsRef = useRef(tabs);
  tabsRef.current = tabs;
  const activeRef = useRef(activeId);
  activeRef.current = activeId;

  const updateSettings = useCallback((fn: (s: Settings) => Settings) => {
    setSettings((prev) => {
      if (!prev) return prev;
      const next = fn(prev);
      void api.saveSettings(next).catch(() => {});
      return next;
    });
  }, []);

  // Restore settings and the tabs that were open last time.
  useEffect(() => {
    (async () => {
      const s = normalize(await api.loadSettings().catch(() => null));
      const home = await api.defaultCloneDir().catch(() => "");
      setCloneDir(s.cloneDir || home);
      const restored: Tab[] = [];
      for (const p of s.openTabs) {
        try {
          restored.push(repoTab(await api.openRepo(p)));
        } catch {
          /* moved or deleted since last run */
        }
      }
      if (restored.length === 0) restored.push(launchpadTab());
      const active = restored.find((t) => t.kind === "repo" && t.path === s.activeTab) ?? restored[0];
      setTabs(restored);
      setActiveId(active.id);
      setSettings(s);
    })();
  }, []);

  // Persist open tabs.
  useEffect(() => {
    if (!settings || tabs.length === 0) return;
    const openTabs = tabs.flatMap((t) => (t.kind === "repo" ? [t.path] : []));
    const active = tabs.find((t) => t.id === activeId);
    const activeTab = active?.kind === "repo" ? active.path : null;
    if (JSON.stringify(openTabs) === JSON.stringify(settings.openTabs) && activeTab === settings.activeTab) return;
    updateSettings((s) => ({ ...s, openTabs, activeTab }));
  }, [tabs, activeId, settings, updateSettings]);

  const remember = useCallback(
    (info: RepoInfo) =>
      updateSettings((s) => ({
        ...s,
        recent: [{ path: info.root, name: info.name, lastOpened: Date.now() }, ...s.recent.filter((r) => r.path !== info.root)].slice(
          0,
          MAX_RECENT,
        ),
      })),
    [updateSettings],
  );

  const showRepo = useCallback(
    (info: RepoInfo) => {
      remember(info);
      const cur = tabsRef.current;
      const existing = cur.find((t) => t.kind === "repo" && t.path === info.root);
      if (existing) {
        setActiveId(existing.id);
        return;
      }
      const tab = repoTab(info);
      const active = cur.find((t) => t.id === activeRef.current);
      setTabs(active?.kind === "launchpad" ? cur.map((t) => (t.id === active.id ? tab : t)) : [...cur, tab]);
      setActiveId(tab.id);
    },
    [remember],
  );

  const openRepo = useCallback(
    async (path: string) => {
      try {
        showRepo(await api.openRepo(path));
      } catch (e) {
        showToast(errorText(e), "error");
      }
    },
    [showRepo],
  );

  const closeTab = (id: string) => {
    const idx = tabs.findIndex((t) => t.id === id);
    let next = tabs.filter((t) => t.id !== id);
    if (next.length === 0) next = [launchpadTab()];
    setTabs(next);
    if (id === activeId) setActiveId(next[Math.min(idx, next.length - 1)].id);
  };

  const newTab = () => {
    const tab = launchpadTab();
    setTabs((t) => [...t, tab]);
    setActiveId(tab.id);
  };

  if (!settings) return <div className="app booting" />;

  const repoTabs = tabs.filter((t): t is Extract<Tab, { kind: "repo" }> => t.kind === "repo");
  const sfUi = settings.smartFeatures ?? DEFAULT_SF_UI;
  const setSfUi = (patch: Partial<SmartFeaturesUi>) =>
    updateSettings((s) => ({ ...s, smartFeatures: { ...(s.smartFeatures ?? DEFAULT_SF_UI), ...patch } }));

  return (
    <div className="app">
      <TabBar
        tabs={tabs}
        activeId={activeId}
        onActivate={setActiveId}
        onClose={closeTab}
        onNew={newTab}
        onSettings={() => setShowSettings((s) => !s)}
        panelOpen={sfUi.panelOpen}
        onTogglePanel={() => setSfUi({ panelOpen: !sfUi.panelOpen })}
      />
      {showSettings && (
        <SettingsPopover
          settings={settings}
          onChange={(patch) => updateSettings((s) => ({ ...s, ...patch }))}
          onClose={() => setShowSettings(false)}
        />
      )}
      <div className="workspace">
        {sfUi.panelOpen && (
          <>
            <SmartFeaturesPanel
              ui={sfUi}
              onUi={setSfUi}
              openPaths={repoTabs.map((r) => r.path)}
              onOpenRepo={(p) => void openRepo(p)}
            />
            <Resizer className="sf-resizer" onDrag={(d) => setSfUi({ width: clamp(sfUi.width + d, 220, 560) })} />
          </>
        )}
        <div className="tab-content">
          {tabs.map((t) => (
            <div key={t.id} className="tab-pane" style={{ display: t.id === activeId ? undefined : "none" }}>
              {t.kind === "launchpad" ? (
                <Launchpad
                  recent={settings.recent}
                  openPaths={repoTabs.map((r) => r.path)}
                  cloneDir={cloneDir}
                  onOpen={(p) => void openRepo(p)}
                  onRemoveRecent={(p) =>
                    updateSettings((s) => ({ ...s, recent: s.recent.filter((r) => r.path !== p) }))
                  }
                  onCloned={(info, dir) => {
                    setCloneDir(dir);
                    updateSettings((s) => ({ ...s, cloneDir: dir }));
                    showToast(`Cloned ${info.name}`, "success");
                    showRepo(info);
                  }}
                />
              ) : (
                <RepoView
                  path={t.path}
                  active={t.id === activeId}
                  autoFetchSeconds={settings.autoFetchSeconds}
                  fetchDelayMs={2000 + repoTabs.findIndex((r) => r.id === t.id) * 4000}
                />
              )}
            </div>
          ))}
        </div>
      </div>
      <Toaster />
    </div>
  );
}
