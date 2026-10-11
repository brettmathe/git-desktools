import { useEffect, useState } from "react";

type Kind = "info" | "success" | "error";
interface Toast {
  id: number;
  text: string;
  kind: Kind;
}

let listeners: ((t: Toast[]) => void)[] = [];
let toasts: Toast[] = [];
let nextId = 1;

function publish() {
  listeners.forEach((l) => l(toasts));
}

export function showToast(text: string, kind: Kind = "info") {
  const toast = { id: nextId++, text, kind };
  toasts = [...toasts, toast];
  publish();
  setTimeout(() => {
    toasts = toasts.filter((t) => t.id !== toast.id);
    publish();
  }, kind === "error" ? 8000 : 3500);
}

export function Toaster() {
  const [items, setItems] = useState<Toast[]>(toasts);
  useEffect(() => {
    listeners.push(setItems);
    return () => {
      listeners = listeners.filter((l) => l !== setItems);
    };
  }, []);
  return (
    <div className="toaster">
      {items.map((t) => (
        <div key={t.id} className={`toast ${t.kind}`}>
          {t.text}
        </div>
      ))}
    </div>
  );
}
