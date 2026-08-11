// App-level warning strip (E12-10 / ADR-0044): a BYOI terminate that failed
// means a provisioned machine MAY STILL BE RUNNING — billed infrastructure,
// not a log line. Dismiss is the only action: the machine is the user's to
// check, and the task it belonged to is already gone.
import { useEffect, useRef, useState } from "react";
import { onFartcodeEvent } from "../lib/tauri";

interface Warning {
  id: number;
  text: string;
}

export default function Warnings() {
  const [warnings, setWarnings] = useState<Warning[]>([]);
  const seq = useRef(0);

  useEffect(() => {
    let unlisten: (() => void) | null = null;
    let disposed = false;
    void onFartcodeEvent((ev) => {
      if (ev.type !== "task:terminate_warning") return;
      const id = seq.current++;
      setWarnings((w) => [...w, { id, text: ev.message }]);
    }).then((u) => {
      if (disposed) u();
      else unlisten = u;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  if (warnings.length === 0) return null;
  return (
    <div className="fc-warnings">
      {warnings.map((w) => (
        <div key={w.id} className="fc-warning" role="alert">
          <span className="fc-warning-text">{w.text}</span>
          <button
            type="button"
            onClick={() => setWarnings((list) => list.filter((x) => x.id !== w.id))}
          >
            dismiss
          </button>
        </div>
      ))}
    </div>
  );
}
