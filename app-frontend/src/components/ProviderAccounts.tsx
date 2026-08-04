// Provider accounts UI (E3-07): credential registry for agent launches.
// Secrets are typed once, stored in the OS keyring server-side, and only
// ever shown as a mask afterwards — this component never handles a secret
// except in the add form's transient input.
import { useCallback, useEffect, useState } from "react";
import {
  ProviderAccountDto,
  ProviderDto,
  addProviderAccount,
  listProviderAccounts,
  listProviders,
  removeProviderAccount,
  setDefaultProviderAccount,
} from "../lib/tauri";

export default function ProviderAccounts() {
  const [providers, setProviders] = useState<ProviderDto[]>([]);
  const [accounts, setAccounts] = useState<ProviderAccountDto[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [providerId, setProviderId] = useState("");
  const [accountId, setAccountId] = useState("");
  const [secret, setSecret] = useState("");
  const [label, setLabel] = useState("");

  const reload = useCallback(async () => {
    try {
      const [ps, accs] = await Promise.all([
        listProviders(),
        listProviderAccounts(null),
      ]);
      setProviders(ps);
      setAccounts(accs);
      if (!providerId && ps.length > 0) setProviderId(ps[0].id);
    } catch (e) {
      setError(String(e));
    }
  }, [providerId]);

  useEffect(() => {
    void reload();
  }, [reload]);

  const add = async () => {
    setError(null);
    if (!providerId || !accountId.trim() || !secret.trim()) return;
    try {
      await addProviderAccount(providerId, accountId.trim(), secret, label.trim() || null);
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

  return (
    <div className="provider-accounts">
      <h3>Provider accounts</h3>
      <p className="hint">
        API keys are stored in the OS keyring — never in the database, logs,
        or UI. The first account per provider becomes the default used for
        agent launches.
      </p>
      {error && <p className="error">{error}</p>}

      {accounts.length > 0 && (
        <ul className="account-list">
          {accounts.map((a) => (
            <li key={a.id} className="account-row">
              <span className="account-provider">{providerName(a.providerId)}</span>
              <span className="account-id">
                {a.label ? `${a.label} (${a.accountId})` : a.accountId}
              </span>
              <span className="account-secret" title="Secret mask">
                {a.maskedSecret}
              </span>
              {a.isDefault ? (
                <span className="account-default">default</span>
              ) : (
                <button onClick={() => void makeDefault(a.id)} title="Make default">
                  make default
                </button>
              )}
              <button
                className="account-remove"
                onClick={() => void remove(a.id)}
                title="Remove account (deletes the keyring secret)"
              >
                &times;
              </button>
            </li>
          ))}
        </ul>
      )}

      <div className="account-add">
        <label>
          Provider
          <select value={providerId} onChange={(e) => setProviderId(e.target.value)}>
            {providers.map((p) => (
              <option key={p.id} value={p.id}>
                {p.name}
              </option>
            ))}
          </select>
        </label>
        <label>
          Account id (username / email)
          <input
            value={accountId}
            onChange={(e) => setAccountId(e.target.value)}
            placeholder="you@example.com"
          />
        </label>
        <label>
          API key
          <input
            type="password"
            autoComplete="off"
            value={secret}
            onChange={(e) => setSecret(e.target.value)}
            placeholder="sk-…"
          />
        </label>
        <label>
          Label (optional)
          <input
            value={label}
            onChange={(e) => setLabel(e.target.value)}
            placeholder="work account"
          />
        </label>
        <button
          className="primary"
          disabled={!providerId || !accountId.trim() || !secret.trim()}
          onClick={() => void add()}
        >
          Add account
        </button>
      </div>
    </div>
  );
}
