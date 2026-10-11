import { createRoot } from "react-dom/client";
import "@xterm/xterm/css/xterm.css";
import "./styles.css";
import App from "./App";

if (import.meta.env.DEV && !("__TAURI_INTERNALS__" in window)) {
  const { installDevBridge } = await import("./devBridge");
  installDevBridge();
}

createRoot(document.getElementById("root")!).render(<App />);
