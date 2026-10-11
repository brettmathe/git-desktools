import { useEffect, useRef } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { api, attachPty, errorText, initPtyBus } from "../api";
import { ClaudeIcon, PlusIcon, TerminalIcon, XIcon } from "./icons";
import { Resizer } from "./Resizer";

export type TermKind = "claude" | "shell";

export interface TermSession {
  key: number;
  kind: TermKind;
  title: string;
}

interface Props {
  cwd: string;
  sessions: TermSession[];
  activeKey: number | null;
  visible: boolean;
  height: number;
  onResize: (delta: number) => void;
  onAdd: (kind: TermKind) => void;
  onCloseSession: (key: number) => void;
  onActivate: (key: number) => void;
  onHide: () => void;
}

export function TerminalPanel(p: Props) {
  return (
    <div className="term-panel" style={{ height: p.height, display: p.visible ? undefined : "none" }}>
      <Resizer dir="y" className="term-resizer" onDrag={(d) => p.onResize(-d)} />
      <div className="term-tabs">
        {p.sessions.map((s) => (
          <div
            key={s.key}
            className={`term-tab${s.key === p.activeKey ? " on" : ""}`}
            onClick={() => p.onActivate(s.key)}
            onAuxClick={(e) => e.button === 1 && p.onCloseSession(s.key)}
          >
            {s.kind === "claude" ? <ClaudeIcon size={12} /> : <TerminalIcon size={12} />}
            {s.title}
            <button
              className="tab-x"
              onClick={(e) => {
                e.stopPropagation();
                p.onCloseSession(s.key);
              }}
            >
              <XIcon size={11} />
            </button>
          </div>
        ))}
        <button className="term-add" onClick={() => p.onAdd("claude")} title="New Claude session">
          <PlusIcon size={12} /> Claude
        </button>
        <button className="term-add" onClick={() => p.onAdd("shell")} title="New shell">
          <PlusIcon size={12} /> Shell
        </button>
        <span className="spacer" />
        <button className="icon-btn" onClick={p.onHide} title="Hide terminal (Ctrl+`)">
          <XIcon size={14} />
        </button>
      </div>
      <div className="term-body">
        {p.sessions.map((s) => (
          <TerminalView key={s.key} cwd={p.cwd} kind={s.kind} visible={p.visible && s.key === p.activeKey} />
        ))}
      </div>
    </div>
  );
}

const THEME = {
  background: "#181a1f",
  foreground: "#d4d7dd",
  cursor: "#d4d7dd",
  selectionBackground: "#3a4a63",
  black: "#1e2128",
  red: "#e06c75",
  green: "#98c379",
  yellow: "#e5c07b",
  blue: "#61afef",
  magenta: "#c678dd",
  cyan: "#56b6c2",
  white: "#d4d7dd",
  brightBlack: "#5c6370",
  brightRed: "#ef7b84",
  brightGreen: "#a9d48a",
  brightYellow: "#f0cf8c",
  brightBlue: "#74bdf5",
  brightMagenta: "#d48ae8",
  brightCyan: "#67c7d3",
  brightWhite: "#ffffff",
};

function TerminalView({ cwd, kind, visible }: { cwd: string; kind: TermKind; visible: boolean }) {
  const hostRef = useRef<HTMLDivElement>(null);
  const termRef = useRef<{ term: Terminal; fit: () => void } | null>(null);

  useEffect(() => {
    const host = hostRef.current!;
    const term = new Terminal({
      fontFamily: 'Menlo, "SF Mono", Monaco, Consolas, "Liberation Mono", monospace',
      fontSize: 13,
      cursorBlink: true,
      scrollback: 10000,
      allowProposedApi: true,
      macOptionIsMeta: true,
      theme: THEME,
    });
    const fitAddon = new FitAddon();
    term.loadAddon(fitAddon);
    term.open(host);

    let ptyId: number | null = null;
    let disposed = false;
    let detach = () => {};

    const fit = () => {
      if (host.offsetParent === null) return; // hidden: xterm can't measure
      try {
        fitAddon.fit();
      } catch {
        /* not measurable yet */
      }
    };
    termRef.current = { term, fit };
    fit();

    const resizeSub = term.onResize(({ cols, rows }) => {
      if (ptyId !== null) void api.ptyResize(ptyId, cols, rows).catch(() => {});
    });
    const dataSub = term.onData((d) => {
      if (ptyId !== null) void api.ptyWrite(ptyId, d).catch(() => {});
    });

    (async () => {
      await initPtyBus();
      const id = await api.ptySpawn(cwd, kind, term.cols, term.rows);
      if (disposed) {
        void api.ptyKill(id);
        return;
      }
      ptyId = id;
      detach = attachPty(id, {
        data: (bytes) => term.write(bytes),
        exit: () => term.write("\r\n\x1b[90m[process exited]\x1b[0m\r\n"),
      });
      term.focus();
    })().catch((e) => term.write(`\x1b[31mCould not start terminal: ${errorText(e)}\x1b[0m\r\n`));

    const ro = new ResizeObserver(() => fit());
    ro.observe(host);

    return () => {
      disposed = true;
      ro.disconnect();
      resizeSub.dispose();
      dataSub.dispose();
      detach();
      if (ptyId !== null) void api.ptyKill(ptyId);
      term.dispose();
      termRef.current = null;
    };
  }, [cwd, kind]);

  useEffect(() => {
    if (!visible) return;
    const raf = requestAnimationFrame(() => {
      termRef.current?.fit();
      termRef.current?.term.focus();
    });
    return () => cancelAnimationFrame(raf);
  }, [visible]);

  return <div className="term-host" ref={hostRef} style={{ display: visible ? undefined : "none" }} />;
}
