import { isMac } from "../util";
import { BranchIcon, GearIcon, LayersIcon, PlusIcon, RocketIcon, XIcon } from "./icons";

export type Tab = { id: string; kind: "launchpad" } | { id: string; kind: "repo"; path: string; name: string };

interface Props {
  tabs: Tab[];
  activeId: string;
  onActivate: (id: string) => void;
  onClose: (id: string) => void;
  onNew: () => void;
  onSettings: () => void;
  panelOpen: boolean;
  onTogglePanel: () => void;
}

export function TabBar({ tabs, activeId, onActivate, onClose, onNew, onSettings, panelOpen, onTogglePanel }: Props) {
  return (
    <div className={`tab-bar${isMac ? " mac" : ""}`} data-tauri-drag-region>
      <button
        className={`icon-btn tab-panel-toggle${panelOpen ? " on" : ""}`}
        onClick={onTogglePanel}
        title={panelOpen ? "Hide Smart Features" : "Show Smart Features"}
      >
        <LayersIcon size={16} />
      </button>
      {tabs.map((t) => (
        <div
          key={t.id}
          className={`tab${t.id === activeId ? " active" : ""}`}
          onMouseDown={(e) => e.button === 0 && onActivate(t.id)}
          onAuxClick={(e) => e.button === 1 && onClose(t.id)}
          title={t.kind === "repo" ? t.path : "Repository Management"}
        >
          {t.kind === "launchpad" ? <RocketIcon size={13} /> : <BranchIcon size={13} />}
          <span className="tab-name">{t.kind === "launchpad" ? "Launchpad" : t.name}</span>
          <button
            className="tab-x"
            onMouseDown={(e) => e.stopPropagation()}
            onClick={() => onClose(t.id)}
            title="Close tab"
          >
            <XIcon size={12} />
          </button>
        </div>
      ))}
      <button className="tab-new" onClick={onNew} title="New tab">
        <PlusIcon size={15} />
      </button>
      <div className="tab-fill" data-tauri-drag-region />
      <button className="icon-btn tab-gear" onClick={onSettings} title="Settings">
        <GearIcon size={16} />
      </button>
    </div>
  );
}
