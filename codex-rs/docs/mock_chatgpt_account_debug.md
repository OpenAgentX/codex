# Local ChatGPT Account Mock Debugging

This document shows how to debug the Codex ChatGPT account flow against a
local Python mock instead of real OpenAI services.

## What This Covers

The mock server in [scripts/mock_chatgpt_account_server.py](/D:/agentx/codex/codex-rs/scripts/mock_chatgpt_account_server.py) supports:

- browser login via `/oauth/authorize`
- device code login via `/api/accounts/deviceauth/*`
- token exchange via `/oauth/token`
- ChatGPT rate limits via `/api/codex/usage`

For browser login, the mock now emits JWT-shaped `id_token` and `access_token`
payloads with `https://api.openai.com/auth` claims so they match Codex's local
login parser.

`account/logout` is still local-only in `codex-rs`: it removes local auth state
and emits the normal account update notifications, but it does not call a remote
logout API.

## Start The Mock Server

From `codex-rs/`:

```bash
python scripts/mock_chatgpt_account_server.py \
  --port 8765 \
  --email debug@example.com \
  --login-password debug-password \
  --plan-type pro \
  --chatgpt-account-id org-debug \
  --additional-limit codex_other:88:30:600
```

For browser login, the default credentials are:

- username: `--login-username` if set, otherwise `--email`
- password: `debug-password` unless overridden with `--login-password`

Useful options:

- `--device-code-auto-approve`: make device code login succeed automatically
- `--device-code-pending-polls 0`: succeed on the first device-code poll
- `--strict-account-header`: require the incoming `chatgpt-account-id` header
- `--login-username`, `--login-password`: set the browser login credentials for `/oauth/authorize`
- `--organization-id`, `--project-id`: override auth claims used by the browser success redirect
- `--no-completed-platform-onboarding` with `--is-org-owner`: trigger the "finish setup" redirect path

Default endpoints after startup:

- issuer: `http://127.0.0.1:8765`
- usage API base URL: `http://127.0.0.1:8765`
- device code approval page: `http://127.0.0.1:8765/codex/device`
- browser logout: `http://127.0.0.1:8765/oauth/logout`

To force the browser flow to ask for credentials again, visit
`/oauth/logout` before starting another `codex login`. You can also provide
`continue_to=/oauth/authorize?...` to clear the session and jump straight back
into the authorize flow.

## Browser Login Against The Mock

Use the existing hidden CLI injection points:

```bash
cargo run -p codex-cli -- login \
  --experimental_issuer http://127.0.0.1:8765 \
  --experimental_client-id local-dev
```

What happens:

1. `codex login` starts its normal localhost callback server.
2. The mock `/oauth/authorize` redirects back with `code` and `state`.
3. `codex login` exchanges that code at the mock `/oauth/token`.
4. The returned fake JWT is persisted to local auth storage.

## Device Code Login Against The Mock

```bash
cargo run -p codex-cli -- login \
  --device-auth \
  --experimental_issuer http://127.0.0.1:8765 \
  --experimental_client-id local-dev
```

Two ways to complete it:

- automatic: start the mock with `--device-code-auto-approve`
- manual: open `http://127.0.0.1:8765/codex/device`, enter the printed user code, and approve it

## Rate Limits And TUI/App-Server Debugging

Point `chatgpt_base_url` at the mock server. Example:

```toml
chatgpt_base_url = "http://127.0.0.1:8765"
```

There is a ready-to-copy example at [scripts/mock-chatgpt-account-config.toml.example](/D:/agentx/codex/codex-rs/scripts/mock-chatgpt-account-config.toml.example).

Recommended workflow for TUI or app-server:

1. Log in once with `codex login` against the mock issuer.
2. Set `chatgpt_base_url` to the mock server.
3. Start TUI or app-server normally.
4. Call `account/read` and `account/rateLimits/read`.

The mock `/api/codex/usage` returns:

- a primary window
- a secondary window
- any repeated `--additional-limit` buckets

## Current Limitation

This setup intentionally avoids adding a new Rust config surface for auth
issuer injection in app-server/TUI.

That means:

- direct CLI login can point at the mock issuer today
- TUI/app-server can consume the resulting local auth and mocked rate limits
- app-server/TUI `account/login/start(type=chatgpt)` does not automatically use
  the mock issuer unless a future Rust change adds that configuration path
