import type { SVGProps } from "react";

type P = SVGProps<SVGSVGElement> & { size?: number };

function Svg({ size = 14, children, ...rest }: P) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
      {...rest}
    >
      {children}
    </svg>
  );
}

export const BranchIcon = (p: P) => (
  <Svg {...p}>
    <circle cx="6" cy="5" r="2.2" />
    <circle cx="6" cy="19" r="2.2" />
    <circle cx="18" cy="7" r="2.2" />
    <path d="M6 7.2v9.6M18 9.2c0 5-12 3-12 7.6" />
  </Svg>
);
export const FolderIcon = (p: P) => (
  <Svg {...p}>
    <path d="M3 6.5A1.5 1.5 0 0 1 4.5 5H9l2 2h8.5A1.5 1.5 0 0 1 21 8.5v9A1.5 1.5 0 0 1 19.5 19h-15A1.5 1.5 0 0 1 3 17.5z" />
  </Svg>
);
export const CloudIcon = (p: P) => (
  <Svg {...p}>
    <path d="M7 18h10a4 4 0 0 0 .6-7.95A6 6 0 0 0 6.1 9.2 4.5 4.5 0 0 0 7 18z" />
  </Svg>
);
export const LaptopIcon = (p: P) => (
  <Svg {...p}>
    <rect x="4" y="5" width="16" height="11" rx="1.5" />
    <path d="M2 19h20" />
  </Svg>
);
export const TagIcon = (p: P) => (
  <Svg {...p}>
    <path d="M3 12V4h8l9 9-8 8z" />
    <circle cx="7.5" cy="8.5" r="1.3" />
  </Svg>
);
export const GitHubIcon = ({ size = 14, ...rest }: P) => (
  <svg width={size} height={size} viewBox="0 0 16 16" fill="currentColor" aria-hidden {...rest}>
    <path d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0 0 16 8c0-4.42-3.58-8-8-8z" />
  </svg>
);
export const TerminalIcon = (p: P) => (
  <Svg {...p}>
    <path d="M4 7l5 5-5 5M12 18h8" />
  </Svg>
);
export const RefreshIcon = (p: P) => (
  <Svg {...p}>
    <path d="M20 11a8 8 0 1 0-2.3 5.7M20 4v7h-7" />
  </Svg>
);
export const FetchIcon = (p: P) => (
  <Svg {...p}>
    <path d="M7 18h10a4 4 0 0 0 .6-7.95A6 6 0 0 0 6.1 9.2 4.5 4.5 0 0 0 7 18z" />
    <path d="M12 10v6M9.5 13.5 12 16l2.5-2.5" />
  </Svg>
);
export const PullIcon = (p: P) => (
  <Svg {...p}>
    <path d="M12 3v13M6 11l6 6 6-6M5 21h14" />
  </Svg>
);
export const PushIcon = (p: P) => (
  <Svg {...p}>
    <path d="M12 17V4M6 10l6-6 6 6M5 21h14" />
  </Svg>
);
export const PlusIcon = (p: P) => (
  <Svg {...p}>
    <path d="M12 5v14M5 12h14" />
  </Svg>
);
export const XIcon = (p: P) => (
  <Svg {...p}>
    <path d="M6 6l12 12M18 6 6 18" />
  </Svg>
);
export const ChevronIcon = ({ open, ...p }: P & { open?: boolean }) => (
  <Svg {...p} style={{ transform: open ? "rotate(90deg)" : undefined, transition: "transform .12s" }}>
    <path d="M9 6l6 6-6 6" />
  </Svg>
);
export const ChevronLeftIcon = (p: P) => (
  <Svg {...p}>
    <path d="M15 6l-6 6 6 6" />
  </Svg>
);
export const SearchIcon = (p: P) => (
  <Svg {...p}>
    <circle cx="11" cy="11" r="6.5" />
    <path d="M20 20l-4.2-4.2" />
  </Svg>
);
export const FileIcon = (p: P) => (
  <Svg {...p}>
    <path d="M6 3h8l4 4v14H6z" />
    <path d="M14 3v4h4" />
  </Svg>
);
export const ListIcon = (p: P) => (
  <Svg {...p}>
    <path d="M8 6h13M8 12h13M8 18h13M3.5 6h.01M3.5 12h.01M3.5 18h.01" />
  </Svg>
);
export const TreeIcon = (p: P) => (
  <Svg {...p}>
    <rect x="3" y="3" width="6" height="5" rx="1" />
    <rect x="14" y="10" width="7" height="4" rx="1" />
    <rect x="14" y="17" width="7" height="4" rx="1" />
    <path d="M6 8v11h8M6 12h8" />
  </Svg>
);
export const RocketIcon = (p: P) => (
  <Svg {...p}>
    <path d="M5 15c-1.5 1.3-2 5-2 5s3.7-.5 5-2M9 15l-3-3c1.5-4.5 6-9 13-9 0 7-4.5 11.5-9 13z" />
    <circle cx="14.5" cy="9.5" r="1.5" />
  </Svg>
);
export const CheckIcon = (p: P) => (
  <Svg {...p}>
    <path d="M5 12.5l4.5 4.5L19 7.5" />
  </Svg>
);
export const PencilIcon = (p: P) => (
  <Svg {...p}>
    <path d="M4 20h4L19 9l-4-4L4 16z" />
  </Svg>
);
export const MinusIcon = (p: P) => (
  <Svg {...p}>
    <path d="M5 12h14" />
  </Svg>
);
export const RenameIcon = (p: P) => (
  <Svg {...p}>
    <path d="M4 12h14M13 7l5 5-5 5" />
  </Svg>
);
export const GearIcon = (p: P) => (
  <Svg {...p}>
    <circle cx="12" cy="12" r="3" />
    <path d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z" />
  </Svg>
);
export const CopyIcon = (p: P) => (
  <Svg {...p}>
    <rect x="9" y="9" width="11" height="11" rx="1.5" />
    <path d="M5 15V5a1 1 0 0 1 1-1h9" />
  </Svg>
);
export const WarningIcon = (p: P) => (
  <Svg {...p}>
    <path d="M12 3 2 20h20zM12 10v4M12 17h.01" />
  </Svg>
);
export const ClaudeIcon = ({ size = 14, ...rest }: P) => (
  <svg width={size} height={size} viewBox="0 0 24 24" aria-hidden {...rest}>
    <g stroke="#d97757" strokeWidth="2.6" strokeLinecap="round">
      <path d="M12 3v18M3 12h18M5.6 5.6l12.8 12.8M18.4 5.6 5.6 18.4" />
    </g>
  </svg>
);

/** Status glyph for a changed file. */
export function StatusIcon({ status, size = 13 }: { status: string; size?: number }) {
  switch (status) {
    case "A":
    case "C":
      return <PlusIcon size={size} className="st st-added" />;
    case "D":
      return <MinusIcon size={size} className="st st-deleted" />;
    case "R":
      return <RenameIcon size={size} className="st st-renamed" />;
    case "U":
      return <WarningIcon size={size} className="st st-conflict" />;
    default:
      return <PencilIcon size={size} className="st st-modified" />;
  }
}
