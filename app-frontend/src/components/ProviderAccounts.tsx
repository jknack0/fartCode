// Provider accounts UI (E3-07): credential registry for agent launches.
// Secrets are typed once, stored in the OS keyring server-side, and only
// ever shown as a mask afterwards — this component never handles a secret
// except in the add form's transient input.
//
// Auth methods (ADR-0034): providers like Claude expose a CLI-login (OAuth
// subscription) method and an API-key method. OAuth accounts store no
// secret — the CLI's own login state authenticates, so fartCode never
// passes ANTHROPIC_API_KEY (which would flip the CLI to API-key billing).
import { useCallback, useEffect, useState } from "react";
import {
  AuthStatusDto,
  ProviderAccountDto,
  ProviderDto,
  addProviderAccount,
  listProviderAccounts,
  listProviders,
  providerAuthLogin,
  providerAuthStatus,
  removeProviderAccount,
  setDefaultProviderAccount,
} from "../lib/tauri";

const LOGIN_TERMINAL_ROWS = 24;
const LOGIN_TERMINAL_COLS = 100;

export default function ProviderAccounts() {
  const [providers, setProviders] = useState<ProviderDto[]>([]);
  const [accounts, setAccounts] = useState<ProviderAccountDto[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [providerId, setProviderId] = useState("");
  const [methodId, setMethodId] = useState<string | null>(null);
  const [accountId, setAccountId] = useState("");
  const [secret, setSecret] = useState("");
  const [label, setLabel] = useState("");
  const [authStatus, setAuthStatus] = useState<AuthStatusDto | null>(null);
  const [authStatusError, setAuthStatusError] = useState<string | null>(null);
  const [checkingStatus, setCheckingStatus] = useState(false);

  const reload = useCallback(async () => {
    try {
      const [ps, accs] = await Promise.all([
        listProviders(),
        listProviderAccounts(null),
      ]);
      setProviders(ps);
      setAccounts(accs);
      if (!providerId && ps.length > 0) {
        setProviderId(ps[0].id);
        setMethodId(ps[0].authMethods[0]?.id ?? null);
      }
    } catch (e) {
      setError(String(e));
    }
  }, [providerId]);

  useEffect(() => {
    void reload();
  }, [reload]);

  const provider = providers.find((p) => p.id === providerId) ?? null;
  const methods = provider?.authMethods ?? [];
  const method = methods.find((m) => m.id === methodId) ?? methods[0] ?? null;
  const isLogin = method?.kind === "cli-login";

  // Poll the CLI login state while a cli-login method is selected.
  const checkStatus = useCallback(
    async (pid: string) => {
      setCheckingStatus(true);
      setAuthStatusError(null);
      try {
        const status = await providerAuthStatus(pid);
        setAuthStatus(status);
        if (status.authenticated && status.account) {
          setAccountId((current) => current || status.account!);
        }
      } catch (e) {
        setAuthStatus(null);
        setAuthStatusError(String(e));
      } finally {
        setCheckingStatus(false);
      }
    },
    [],
  );

  useEffect(() => {
    if (providerId && isLogin) {
      void checkStatus(providerId);
    } else {
      setAuthStatus(null);
      setAuthStatusError(null);
    }
  }, [providerId, isLogin, checkStatus]);

  const onProviderChange = (id: string) => {
    setProviderId(id);
    const p = providers.find((x) => x.id === id);
    setMethodId(p?.authMethods[0]?.id ?? null);
    setAccountId("");
    setSecret("");
    setAuthStatus(null);
    setAuthStatusError(null);
  };

  const onMethodChange = (id: string) => {
    setMethodId(id);
    setAccountId("");
    setSecret("");
  };

  const signIn = async () => {
    setError(null);
    try {
      await providerAuthLogin(providerId, null, LOGIN_TERMINAL_ROWS, LOGIN_TERMINAL_COLS);
      // The login flow runs in its own terminal; poll status so the form
      // flips to authenticated once the OAuth handshake completes.
      await checkStatus(providerId);
    } catch (e) {
      setError(String(e));
    }
  };

  const add = async () => {
    setError(null);
    if (!providerId || !accountId.trim()) return;
    if (!isLogin && !secret.trim()) return;
    try {
      await addProviderAccount(
        providerId,
        accountId.trim(),
        isLogin ? "" : secret.trim(),
        label.trim() || null,
        isLogin ? method!.id : method?.id ?? null,
      );
      setAccountId("");
      setSecret("");
      setLabel("");
      await reload();
    } catch (e) {
      setError(String(e));
    }
  };

  const remove = async (id: string) => {
    setError(null);
    try {
      await removeProviderAccount(id);
      await reload();
    } catch (e) {
      setError(String(e));
    }
  };

  const makeDefault = async (id: string) => {
    setError(null);
    try {
      await setDefaultProviderAccount(id);
      await reload();
    } catch (e) {
      setError(String(e));
    }
  };

  const providerName = (id: string) =>
    providers.find((p) => p.id === id)?.name ?? id;

  const methodBadge = (authMethod: string | null) => {
    if (authMethod === "claude-login") return "OAuth sub";
    if (authMethod === "anthropic-api-key") return "API key";
    if (authMethod) return authMethod;
    return null;
  };

  return (
    <div className="fc-accounts">
      <div className="fc-set-group-label">Provider accounts</div>
      <p className="fc-set-hint">
        API keys are stored in the OS keyring — never in the database, logs,
        or UI. The first account per provider becomes the default used for
        agent launches. OAuth accounts (e.g. Claude "Sign in with Claude
        Code") use your subscription — no API charges — and fartCode never
        passes an API key for them.
      </p>
      {error && <p className="fc-set-error">{error}</p>}

      {accounts.length > 0 && (
        <ul className="fc-acct-list">
          {accounts.map((a) => {
            const badge = methodBadge(a.authMethod);
            return (
              <li key={a.id} className="fc-acct-row">
                <span className="fc-acct-name">
                  {providerName(a.providerId)}
                  {a.isDefault && <span className="fc-tag-green">default</span>}
                </span>
                <span className="fc-acct-meta">
                  {a.label ? `${a.label} (${a.accountId})` : a.accountId}
                  {badge ? ` · ${badge}` : ""}
                  {a.authMethod === "claude-login" ? "" : ` · ${a.maskedSecret}`}
                  {!a.isDefault && (
                    <>
                      {" · "}
                      <button
                        className="fc-acct-action"
                        onClick={() => void makeDefault(a.id)}
                        title="Make default"
                      >
                        make default
                      </button>
                    </>
                  )}
                  {" · "}
                  <button
                    className="fc-acct-action remove"
                    onClick={() => void remove(a.id)}
                    title="Remove account (deletes the keyring secret)"
                  >
                    ✗
                  </button>
                </span>
              </li>
            );
          })}
        </ul>
      )}

      <div className="fc-acct-editor">
        <label className="fc-acct-field">
          <span className="fc-acct-field-name">provider</span>
          <select value={providerId} onChange={(e) => onProviderChange(e.target.value)}>
            {providers.map((p) => (
              <option key={p.id} value={p.id}>
                {p.name}
              </option>
            ))}
          </select>
        </label>

        {methods.length > 0 && (
          <label className="fc-acct-field">
            <span className="fc-acct-field-name">auth method</span>
            <select value={method?.id ?? ""} onChange={(e) => onMethodChange(e.target.value)}>
              {methods.map((m) => (
                <option key={m.id} value={m.id}>
                  {m.name}
                </option>
              ))}
            </select>
          </label>
        )}

        {isLogin ? (
          <>
            <div className="fc-acct-field">
              <span className="fc-acct-field-name">login status</span>
              <span className="fc-acct-status">
                {checkingStatus ? (
                  "checking…"
                ) : authStatus?.authenticated ? (
                  <span className="fc-acct-status-ok">
                    signed in as {authStatus.account ?? "your Claude account"} (OAuth)
                  </span>
                ) : (
                  <>
                    not signed in via the CLI{authStatusError ? ` — ${authStatusError}` : ""}
                    {" · "}
                    <button
                      className="fc-acct-editor-action signin"
                      onClick={() => void signIn()}
                    >
                      sign in with Claude Code
                    </button>
                  </>
                )}
              </span>
            </div>
            <label className="fc-acct-field">
              <span className="fc-acct-field-name">account id (from your CLI login)</span>
              <input
                value={accountId}
                onChange={(e) => setAccountId(e.target.value)}
                onKeyDown={(e) => e.key === "Enter" && void add()}
                placeholder="you@example.com"
              />
            </label>
            <label className="fc-acct-field">
              <span className="fc-acct-field-name">label (optional)</span>
              <input
                value={label}
                onChange={(e) => setLabel(e.target.value)}
                onKeyDown={(e) => e.key === "Enter" && void add()}
                placeholder="work account"
              />
            </label>
            <div className="fc-acct-editor-keys">
              <button
                className="fc-acct-editor-action"
                disabled={!providerId || !accountId.trim() || !authStatus?.authenticated}
                onClick={() => void add()}
              >
                ↵ add account
              </button>
            </div>
          </>
        ) : (
          <>
            <label className="fc-acct-field">
              <span className="fc-acct-field-name">account id (username / email)</span>
              <input
                value={accountId}
                onChange={(e) => setAccountId(e.target.value)}
                onKeyDown={(e) => e.key === "Enter" && void add()}
                placeholder="you@example.com"
              />
            </label>
            <label className="fc-acct-field">
              <span className="fc-acct-field-name">api key</span>
              <input
                type="password"
                autoComplete="off"
                value={secret}
                onChange={(e) => setSecret(e.target.value)}
                onKeyDown={(e) => e.key === "Enter" && void add()}
                placeholder="sk-…"
              />
            </label>
            <label className="fc-acct-field">
              <span className="fc-acct-field-name">label (optional)</span>
              <input
                value={label}
                onChange={(e) => setLabel(e.target.value)}
                onKeyDown={(e) => e.key === "Enter" && void add()}
                placeholder="work account"
              />
            </label>
            <div className="fc-acct-editor-keys">
              <button
                className="fc-acct-editor-action"
                disabled={!providerId || !accountId.trim() || !secret.trim()}
                onClick={() => void add()}
              >
                ↵ add account
              </button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}
