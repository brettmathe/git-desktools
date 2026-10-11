import { useState } from "react";
import { ClaudeIcon, GitHubIcon } from "./icons";
import { hashColor } from "../util";

const failed = new Set<string>();

function avatarUrl(email: string, gravatar: string): string | null {
  const gh = /^(\d+)\+[^@]+@users\.noreply\.github\.com$/i.exec(email);
  if (gh) return `https://avatars.githubusercontent.com/u/${gh[1]}?s=64&v=4`;
  if (!email) return null;
  return `https://www.gravatar.com/avatar/${gravatar}?s=64&d=404`;
}

function initials(name: string): string {
  const parts = name.trim().split(/\s+/).filter(Boolean);
  if (parts.length === 0) return "?";
  if (parts.length === 1) return parts[0].slice(0, 2).toUpperCase();
  return (parts[0][0] + parts[parts.length - 1][0]).toUpperCase();
}

interface Props {
  name: string;
  email: string;
  gravatar: string;
  size?: number;
  title?: string;
}

export function Avatar({ name, email, gravatar, size = 22, title }: Props) {
  const [, rerender] = useState(0);
  const style = { width: size, height: size };
  const lower = email.toLowerCase();

  if (lower === "noreply@anthropic.com") {
    return (
      <span className="avatar special claude" style={style} title={title ?? name}>
        <ClaudeIcon size={size * 0.62} />
      </span>
    );
  }
  if (lower === "noreply@github.com") {
    return (
      <span className="avatar special github" style={style} title={title ?? name}>
        <GitHubIcon size={size * 0.66} />
      </span>
    );
  }

  const url = avatarUrl(email, gravatar);
  if (url && !failed.has(url)) {
    return (
      <img
        className="avatar"
        src={url}
        style={style}
        title={title ?? name}
        alt=""
        draggable={false}
        loading="lazy"
        onError={() => {
          failed.add(url);
          rerender((n) => n + 1);
        }}
      />
    );
  }
  return (
    <span
      className="avatar initials"
      style={{ ...style, background: hashColor(email || name), fontSize: Math.round(size * 0.4) }}
      title={title ?? name}
    >
      {initials(name)}
    </span>
  );
}
