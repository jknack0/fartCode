// OS notifications for the "agent needs you" events (issue #140).
// Wires the three backend event channels to the OS notification plugin,
// gated by the `notifications.os_notifications` app setting (default true)
// and by window focus — the point is to reach the user while the app is in
// the background, so a focused window stays silent.
import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from "@tauri-apps/plugin-notification";
import {
  getAppSetting,
  onAcpPermissionRequest,
  onFartcodeEvent,
} from "./tauri";
import { wireEvents } from "./wireEvents";

let osNotifications = true;

/** The settings toggle writes through here so the wiring sees the change
 * immediately (no need to round-trip the event channel). */
export function setOsNotifications(enabled: boolean): void {
  osNotifications = enabled;
}

/** One-time read of the persisted toggle (app boot / restart). */
export async function loadNotificationSetting(): Promise<void> {
  try {
    const group = (await getAppSetting("notifications")) as {
      osNotifications?: boolean;
    };
    osNotifications = group?.osNotifications ?? true;
  } catch {
    osNotifications = true; // settings unreadable — stay permissive
  }
}

function shouldNotify(): boolean {
  // `document.hasFocus()` is false exactly when the webview window is not
  // the focused window — the "app is in the background" case the issue
  // names. Focused users already see the permission card / board dot.
  return osNotifications && !document.hasFocus();
}

async function notify(title: string, body?: string): Promise<void> {
  try {
    let granted = await isPermissionGranted();
    if (!granted) {
      granted = (await requestPermission()) === "granted";
    }
    if (granted) {
      sendNotification({ title, body });
    }
  } catch {
    // Best-effort: a notification failure must never break the app.
  }
}

export function wireNotificationEvents(): () => void {
  const unwires = [
    // permission-prompt: an agent is blocked on an approval gate.
    wireEvents(onAcpPermissionRequest, ({ pending }) => {
      if (!shouldNotify()) return;
      const tool =
        pending?.toolCall?.title ?? pending?.toolCall?.kind ?? "an action";
      void notify("Agent needs permission", `Waiting for approval to run ${tool}.`);
    }),
    // needs-you + settle: task moved to review / a step settled on a
    // human gate.
    wireEvents(onFartcodeEvent, (event) => {
      if (!shouldNotify()) return;
      if (event.type === "task:status_changed" && event.status === "review") {
        void notify("Agent needs you", "A task is waiting for review.");
      } else if (event.type === "step:settled") {
        void notify("Step settled", "A board step finished and is waiting for you.");
      }
    }),
  ];
  return () => {
    for (const unwire of unwires) unwire();
  };
}
