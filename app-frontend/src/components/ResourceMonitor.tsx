// Resource monitor panel (E1-09): live CPU/memory samples, shown when
// enabled (toggle lives in the palette). Polls every second while open.
import { useEffect, useState } from "react";
import {
  ResourceSampleDto,
  getResourceMonitorEnabled,
  resourceSample,
} from "../lib/tauri";
import { useUi } from "../store/ui";

export default function ResourceMonitor() {
  const open = useUi((s) => s.resourceOpen);
  const setOpen = useUi((s) => s.setResourceOpen);
  const [enabled, setEnabled] = useState(false);
  const [sample, setSample] = useState<ResourceSampleDto | null>(null);

  useEffect(() => {
    // Re-fetch whenever the panel is opened: the palette's "Toggle resource
    // monitor" command flips the persisted setting and opens the panel in one
    // action, so reading only at mount would render nothing after a toggle.
    getResourceMonitorEnabled().then(setEnabled).catch(() => {});
  }, [open]);

  useEffect(() => {
    if (!open || !enabled) return;
    let cancelled = false;
    const tick = async () => {
      try {
        const s = await resourceSample();
        if (!cancelled) setSample(s);
      } catch {
        /* sampling failures are non-fatal */
      }
    };
    tick();
    const interval = setInterval(tick, 1000);
    return () => {
      cancelled = true;
      clearInterval(interval);
    };
  }, [open, enabled]);

  if (!open || !enabled) return null;

  const pct = sample ? Math.round(sample.cpuPercent) : 0;
  return (
    <div className="resource-panel">
      <div className="resource-header">
        <span>Resource monitor</span>
        <button onClick={() => setOpen(false)}>✕</button>
      </div>
      {sample ? (
        <div className="resource-body">
          <div className="meter">
            <span className="meter-label">CPU</span>
            <div className="meter-bar">
              <div className="meter-fill" style={{ width: `${pct}%` }} />
            </div>
            <span className="meter-value">{pct}%</span>
          </div>
          <div className="meter">
            <span className="meter-label">MEM</span>
            <div className="meter-bar">
              <div
                className="meter-fill"
                style={{
                  width: `${Math.min(100, (sample.memUsedMb / sample.memTotalMb) * 100)}%`,
                }}
              />
            </div>
            <span className="meter-value">
              {sample.memUsedMb} / {sample.memTotalMb} MB
            </span>
          </div>
        </div>
      ) : (
        <p className="muted">Sampling…</p>
      )}
    </div>
  );
}
