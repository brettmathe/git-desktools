import type { MouseEvent } from "react";

interface Props {
  dir?: "x" | "y";
  className?: string;
  onDrag: (delta: number) => void;
}

/** Thin drag handle; reports pointer movement deltas while dragging. */
export function Resizer({ dir = "x", className = "", onDrag }: Props) {
  const onMouseDown = (e: MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    let last = dir === "x" ? e.clientX : e.clientY;
    const move = (ev: globalThis.MouseEvent) => {
      const cur = dir === "x" ? ev.clientX : ev.clientY;
      onDrag(cur - last);
      last = cur;
    };
    const up = () => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
      document.body.classList.remove("resizing", `resizing-${dir}`);
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
    document.body.classList.add("resizing", `resizing-${dir}`);
  };
  return <div className={`resizer resizer-${dir} ${className}`} onMouseDown={onMouseDown} />;
}

export const clamp = (v: number, min: number, max: number) => Math.min(max, Math.max(min, v));
