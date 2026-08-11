// SSH connections (E12-03 profiles, E12-06 lifecycle).
//
// The panel answers two questions the backend already knows and nothing was
// asking: which hosts do I have, and what is each one doing right now. State
// arrives as events (`ssh:state_changed`), so a reconnect ladder counts down
// in place instead of waiting for a refresh; `ssh:health_changed` raises the
// MaxSessions note, which is a HOST configuration problem — reconnecting
// cannot fix it, so it reads as advice, not as an error.
//
// Secrets are typed once and stored in the OS keyring server-side. A profile
// that has one shows “stored” and a blank field means “keep it”.
import { useCallback, useEffect, useRef, useState } from "react";
import {
  SshConnectionDto,
  SshConnectionState,
  onFartcodeEvent,
  sshConnect,
  sshConnectionDelete,
  sshConnectionList,
  sshConnectionSave,
  sshConnectionStates,
  sshDisconnect,
} from "../lib/tauri";

/** What a row shows while a connection is doing something. */
interface LiveState {
  state: SshConnectionState;
  attempt: number | null;
  delayMs: number | null;
  error: string | null;
  degraded: boolean;
}

const IDLE: LiveState = {
  state: "disconnected",
  attempt: null,
  delayMs: null,
  error: null,
  degraded: false,
};

const STATE_LABEL: Record<SshConnectionState, string> = {
  connecting: "connecting",
  connected: "connected",
  reconnecting: "reconnecting",
  disconnected: "offline",
  error: "unreachable",
};

/** Row summary: the state, plus what the backoff ladder is doing (E12-06). */
export function stateSummary(live: LiveState): string {
  if (live.state === "reconnecting" && live.attempt !== null) {
    const seconds = Math.round((live.delayMs ?? 0) / 1000);
    return `reconnecting · ${seconds}s (${live.attempt}/5)`;
  }
  return STATE_LABEL[live.state];
}

const EMPTY_FORM = {
  id: null as string | null,
  name: "",
  host: "",
  port: "22",
  username: "",
  authType: "agent",
  privateKeyPath: "",
  alias: "",
  proxyJump: "",
  forwardAgent: false,
  secret: "",
  hasSecret: false,
};

export default function SshConnections() {
  const [connections, setConnections] = useState<SshConnectionDto[]>([]);
  const [live, setLive] = useState<Record<string, LiveState>>({});
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [form, setForm] = useState(EMPTY_FORM);
  const [editing, setEditing] = useState(false);
  const mounted = useRef(true);

  const reload = useCallback(async () => {
    try {
      const [list, states] = await Promise.all([
        sshConnectionList(),
        sshConnectionStates(),
      ]);
      if (!mounted.current) return;
      setConnections(list);
      setLive((current) => {
        const next = { ...current };
        for (const s of states) {
          next[s.connectionId] = {
            ...(next[s.connectionId] ?? IDLE),
            state: s.state,
            degraded: s.degraded,
          };
        }
        return next;
      });
    } catch (e) {
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    mounted.current = true;
    void reload();
    const unlisten = onFartcodeEvent((event) => {
      if (event.type === "ssh:state_changed") {
        setLive((current) => ({
          ...current,
          [event.connectionId]: {
            ...(current[event.connectionId] ?? IDLE),
            state: event.state,
            attempt: event.attempt,
            delayMs: event.delayMs,
            error: event.error,
          },
        }));
      } else if (event.type === "ssh:health_changed") {
        setLive((current) => ({
          ...current,
          [event.connectionId]: {
            ...(current[event.connectionId] ?? IDLE),
            degraded: event.degraded,
          },
        }));
      }
    });
    return () => {
      mounted.current = false;
      void unlisten.then((off) => off()).catch(() => {});
    };
  }, [reload]);

  const run = useCallback(
    async (id: string, action: () => Promise<unknown>) => {
      setBusy(id);
      setError(null);
      try {
        await action();
      } catch (e) {
        setError(String(e));
      } finally {
        setBusy(null);
      }
    },
    [],
  );

  const edit = (c: SshConnectionDto) => {
    setForm({
      id: c.id,
      name: c.name,
      host: c.host,
      port: String(c.port),
      username: c.username,
      authType: c.authType,
      privateKeyPath: c.privateKeyPath ?? "",
      alias: c.alias ?? "",
      proxyJump: c.proxyJump ?? "",
      forwardAgent: c.forwardAgent,
      secret: "",
      hasSecret: c.hasSecret,
    });
    setEditing(true);
  };

  const save = async () => {
    setError(null);
    try {
      await sshConnectionSave({
        id: form.id,
        name: form.name.trim() || form.host.trim(),
        host: form.host,
        port: Number(form.port) || 22,
        username: form.username,
        authType: form.authType,
        privateKeyPath: form.privateKeyPath,
        useAgent: form.authType === "agent",
        alias: form.alias,
        proxyJump: form.proxyJump,
        forwardAgent: form.forwardAgent,
        secret: form.secret,
      });
      setForm(EMPTY_FORM);
      setEditing(false);
      await reload();
    } catch (e) {
      setError(String(e));
    }
  };

  const degraded = connections.filter((c) => live[c.id]?.degraded);

  return (
    <div className="fc-accounts">
      <div className="fc-set-group-label">SSH connections</div>
      <p className="fc-set-hint">
        Hosts for remote projects and terminals. Passwords and key passphrases
        are stored in the OS keyring, never in the database.
      </p>

      {error && <p className="fc-set-error">{error}</p>}

      {degraded.length > 0 && (
        <div className="fc-conn-warn">
          <span className="fc-conn-warn-head">
            {degraded.map((c) => c.name).join(", ")} refused a new channel
          </span>
          <span className="fc-conn-warn-body">
            The connection is alive — the host has run out of sessions.
            OpenSSH allows <code>MaxSessions</code> 10 by default; raise it in{" "}
            <code>/etc/ssh/sshd_config</code> and reload sshd:
          </span>
          <code className="fc-conn-warn-code">MaxSessions 100</code>
        </div>
      )}

      {connections.length === 0 ? (
        <p className="fc-set-hint">No connections yet.</p>
      ) : (
        <ul className="fc-acct-list">
          {connections.map((c) => {
            const state = live[c.id] ?? IDLE;
            const connected = state.state === "connected";
            return (
              <li key={c.id} className="fc-acct-row">
                <span className="fc-acct-name">
                  {c.name}
                  <span className={`fc-conn-state ${state.state}`}>
                    {stateSummary(state)}
                  </span>
                  {c.alias && <span className="fc-tag-green">{c.alias}</span>}
                </span>
                <span className="fc-acct-meta">
                  <span className="fc-conn-target">
                    {c.username}@{c.host}
                    {c.port !== 22 ? `:${c.port}` : ""}
                  </span>
                  <button
                    className="fc-acct-action"
                    disabled={busy === c.id}
                    onClick={() =>
                      void run(c.id, () =>
                        connected ? sshDisconnect(c.id) : sshConnect(c.id),
                      )
                    }
                  >
                    {connected ? "disconnect" : "connect"}
                  </button>
                  <button className="fc-acct-action" onClick={() => edit(c)}>
                    edit
                  </button>
                  <button
                    className="fc-acct-action remove"
                    onClick={() =>
                      void run(c.id, async () => {
                        await sshConnectionDelete(c.id);
                        await reload();
                      })
                    }
                  >
                    remove
                  </button>
                </span>
              </li>
            );
          })}
        </ul>
      )}

      {state_error(live) && <p className="fc-set-error">{state_error(live)}</p>}

      {editing ? (
        <div className="fc-acct-editor">
          <label className="fc-acct-field">
            <span className="fc-acct-field-name">name</span>
            <input
              value={form.name}
              placeholder="build box"
              onChange={(e) => setForm({ ...form, name: e.target.value })}
            />
          </label>
          <label className="fc-acct-field">
            <span className="fc-acct-field-name">host</span>
            <input
              value={form.host}
              placeholder="10.0.0.4"
              onChange={(e) => setForm({ ...form, host: e.target.value })}
            />
          </label>
          <label className="fc-acct-field">
            <span className="fc-acct-field-name">user</span>
            <input
              value={form.username}
              placeholder="deploy"
              onChange={(e) => setForm({ ...form, username: e.target.value })}
            />
          </label>
          <label className="fc-acct-field">
            <span className="fc-acct-field-name">port</span>
            <input
              value={form.port}
              inputMode="numeric"
              onChange={(e) => setForm({ ...form, port: e.target.value })}
            />
          </label>
          <label className="fc-acct-field">
            <span className="fc-acct-field-name">auth</span>
            <select
              value={form.authType}
              onChange={(e) => setForm({ ...form, authType: e.target.value })}
            >
              <option value="agent">agent</option>
              <option value="password">password</option>
              <option value="key-file">key file</option>
            </select>
          </label>
          {form.authType === "key-file" && (
            <label className="fc-acct-field">
              <span className="fc-acct-field-name">key path</span>
              <input
                value={form.privateKeyPath}
                placeholder="~/.ssh/id_ed25519"
                onChange={(e) =>
                  setForm({ ...form, privateKeyPath: e.target.value })
                }
              />
            </label>
          )}
          {form.authType !== "agent" && (
            <label className="fc-acct-field">
              <span className="fc-acct-field-name">
                {form.authType === "password" ? "password" : "passphrase"}
              </span>
              <input
                type="password"
                value={form.secret}
                placeholder={form.hasSecret ? "stored — blank keeps it" : ""}
                onChange={(e) => setForm({ ...form, secret: e.target.value })}
              />
            </label>
          )}
          <label className="fc-acct-field">
            <span className="fc-acct-field-name">ssh config alias</span>
            <input
              value={form.alias}
              placeholder="optional — resolved with ssh -G"
              onChange={(e) => setForm({ ...form, alias: e.target.value })}
            />
          </label>
          <label className="fc-acct-field">
            <span className="fc-acct-field-name">proxy jump</span>
            <input
              value={form.proxyJump}
              placeholder="optional — bastion@edge"
              onChange={(e) => setForm({ ...form, proxyJump: e.target.value })}
            />
          </label>
          <label className="fc-acct-field">
            <span className="fc-acct-field-name">forward agent</span>
            <input
              type="checkbox"
              checked={form.forwardAgent}
              onChange={(e) =>
                setForm({ ...form, forwardAgent: e.target.checked })
              }
            />
          </label>
          <div className="fc-acct-editor-action">
            <button onClick={() => void save()}>
              {form.id ? "save" : "add connection"}
            </button>
            <button
              className="fc-acct-action"
              onClick={() => {
                setForm(EMPTY_FORM);
                setEditing(false);
              }}
            >
              cancel
            </button>
          </div>
        </div>
      ) : (
        <div className="fc-acct-editor-action">
          <button onClick={() => setEditing(true)}>add connection</button>
        </div>
      )}
    </div>
  );
}

/** The last connection error worth showing, if any. */
function state_error(live: Record<string, LiveState>): string | null {
  const failed = Object.values(live).find(
    (s) => s.state === "error" && s.error,
  );
  return failed?.error ?? null;
}
