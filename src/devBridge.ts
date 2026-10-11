// Dev-only: when the UI runs in a plain browser (no Tauri), route `invoke` calls
// to the test-only HTTP bridge in src-tauri/src/dev_bridge.rs.
import { mockIPC } from "@tauri-apps/api/mocks";

const BRIDGE = `http://127.0.0.1:${new URLSearchParams(location.search).get("bridge") ?? "1421"}`;

export function installDevBridge() {
  mockIPC(
    async (cmd, args) => {
      const res = await fetch(`${BRIDGE}/cmd/${cmd}`, { method: "POST", body: JSON.stringify(args ?? {}) });
      const body = await res.json();
      if (!res.ok) throw body;
      return body;
    },
    { shouldMockEvents: true },
  );
}
