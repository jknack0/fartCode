-- Provider auth methods (E3-07 login methods, ADR-0034): an account can
-- authenticate via a CLI-managed login (OAuth subscription, e.g.
-- `claude auth login`) instead of an API key. NULL = legacy rows created
-- before this column existed — they behave as api-key accounts.
ALTER TABLE provider_accounts ADD COLUMN auth_method TEXT;
