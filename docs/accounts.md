# Avelor — Accounts

> The accounts the app works with: AI providers — their files, secrets and
> states, and the Accounts page.

## Accounts in a vault

Accounts belong to a vault. Each account is a YAML file in the vault's
`.avelor/accounts/` folder (`accounts.md#account-files`); opening another vault
switches to its accounts (`accounts::open`, on every vault open), and
without an open vault there is nowhere to keep an account — the Accounts
page says so. A file without a valid `id` is skipped (and
logged); keys the app doesn't know are kept.

## Secrets <a id="secrets"></a>

An account's secret (an API key) stays with the app, never in a vault: it
is a file in the configuration folder (`%LOCALAPPDATA%\Avelor\`,
`architecture.md#storage`) named after the account's id, `<id>.key`,
DPAPI-encrypted for the current Windows user (`av-utils/secret.rs`). On Linux the file is in
`$XDG_CONFIG_HOME/avelor`, holds the key as it is, and only the
user can read it (0600, `fs::write_private`). So no file in the vault names
its secret, and a backup of the vault never holds one. Only the key's ends
reach the page (`sk-…4f2a`).

## Connection status <a id="connection"></a>

`ConnectionStatus` (`config_files.rs`):

- **connected** — the secret can be read;
- **reconnect** — the secret is missing or can't be decrypted (the vault
  opened on another computer, or as another user), and the file isn't
  marked `disconnected`: the account shows as "needs a key";
- **disconnected** — the user disconnected it: the file is marked
  `disconnected`, which tells a deliberate disconnect from a secret
  missing on this computer, and the secret is deleted. Connecting again
  removes the mark.

Removing an account deletes its file and its secret. A local Ollama
account needs no key; its status is whether the server answers:
`running`, `not running`, or `unknown` (not checked yet).

## Account files <a id="account-files"></a>

Files are named `<kind>_<name>.yaml`: a new AI account
`<provider>.yaml`, then `<provider>_2.yaml`, …. The kind of a file is its
top-level key. One pass over `.avelor/accounts/` when a vault opens
(`accounts::load`) hands each file to the loader of its kind.

Every account file has:

* `id`: UUIDv7, unique identifier for the account

### AI account file <a id="ai-account-file"></a>

* `anthropic`, `openai`, `mistral`, `gemini`, `ollama`: an AI provider's
  account (the block is named after the provider; `claude` is read as
  `anthropic`)
  * `address`: where the API lives, if not at the provider's default
    address (Ollama: `http://localhost:11434`)
  * `offered`: the models offered wherever a model is picked (ticked on
    the Accounts page)
  * `spend-limit`: USD a month
  * `connected`: when the account was first connected
  * `disconnected`: when its key was forgotten

```yaml
id: 0193a7c1-4b0e-7c1a-9d2f-3a51c8e05be0
openai:
  offered: [gpt-5, gpt-5-mini, gpt-5-nano]
  spend-limit: 10
  connected: 2026-09-30T10:00:00Z
```

## The Accounts page <a id="accounts-page"></a>

Workspace → Accounts (`features/workspace/accounts.js`) lists the open vault's
accounts with a detail pane.

- **AI providers**: the AI provider menu offers Anthropic, OpenAI, Mistral
  and Gemini with an API key, and Ollama by its address. A key or an address
  is checked by listing the provider's models before it is stored; until
  then it stays an input. The pane lists the models with their price per
  million tokens: ticking one offers it in the features' pickers. An Ollama
  account shows whether the server answers. Disconnecting forgets the key
  but keeps the account and its id, marked `disconnected`; removing
  deletes its files (Ollama accounts are removed directly).
- The page widgets the Workspace pages share are `ui/`'s (`ui/page.js`,
  `ui/field.js`).

## Flows

| # | Flow | Trigger → page | Host → events | Result |
|---|---|---|---|---|
| H1 | Open the page | Workspace → Accounts → `openVaultOp('accounts')` → `renderAccounts` | — (the accounts came with the vault: `accounts`) | list + detail pane; without a vault, a hint |
| I1 | Add an AI provider | "AI provider" menu → draft pane → key or address → Save → `aiConnect` | `connect_ai {provider, key \| address}` (channel `connect_ai`) → models listed with the new values on a worker → only then stored in `.avelor/accounts/` (the key as `<id>.key`) → `account`, `ok {id, models}` | card with models |
| I2 | Models, refresh, test | pane renders, Refresh, Test | `list_ai_models {account}` → `account` (Ollama: running?), `ok {models}` | — |
| I3 | Offer a model | tick | `set_ai_settings {account, offered}` → `account`, `ok` | pickers offer it |
| I4 | Spend limit | edit field | `set_ai_settings {account, spend_limit}` → `account`, `ok` | — |
| I5 | Replace key | edit key | `connect_ai {account, key}` | — |
| I6 | Disconnect / remove | two clicks within 3 s | `disconnect_account` / `remove_account` | — |

## Design <a id="design"></a>

What the account files share is in `config_files.rs`:

```rust
pub struct SecretFile(PathBuf);         // <config folder>/<account id>.key
impl SecretFile {
    pub fn of(secrets: &Path, id: Id, ext: &str) -> Self;
    pub fn read(&self) -> Result<String>;          // FileNotFound, or DPAPI fails
    pub fn write(&self, secret: &str) -> Result<()>; // DPAPI, write_private
    pub fn remove(&self) -> Result<()>;
}
pub enum ConnectionStatus { Connected, Disconnected, Reconnect }
impl ConnectionStatus { pub fn of(secret_read: bool, disconnected: bool) -> Self; }
pub fn write_yaml(path: &Path, value: &impl Serialize) -> Result<()>;
```

plus `load_config_files` and `free_stem` (the next free file name).
`AiAccounts` (`ai_accounts.rs`) is the sub-state that owns the files;
`accounts.rs` does the pass on vault open, disconnect, remove, the events
`accounts` and `account_removed`.

av-llm's `Client` speaks to Anthropic, OpenAI, Mistral, Gemini and Ollama,
with the price table (`architecture.md#crates`). What the app knows of
each provider is one table, `PROVIDERS` (`av-llm/types.rs`): its name in
files (`claude` as an alias of `anthropic`), the name the Accounts page
shows, the default address, whether it takes a key (Ollama is addressed
instead), whether it keeps uploads, and how the key is sent. The account files' kinds, the
key's checks and the error wording read it there.
