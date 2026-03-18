# Local ChatGPT Account Mock Debugging

This document shows how to debug the Codex ChatGPT account flow against a
local Python mock instead of real OpenAI services.

## What This Covers

The mock server in [scripts/mock_chatgpt_account_server.py](/D:/agentx/codex/codex-rs/scripts/mock_chatgpt_account_server.py) supports:

- browser login via `/oauth/authorize`
- device code login via `/api/accounts/deviceauth/*`
- token exchange via `/oauth/token`
- token refresh via `/oauth/token` with `grant_type=refresh_token`
- Responses API via `/backend-api/codex/responses`
- remote models via `/models`, `/v1/models`, and `/backend-api/codex/models`
- ChatGPT backend helpers via `/backend-api/wham/usage`, `/backend-api/wham/tasks*`,
  `/backend-api/wham/config/requirements`, and `/backend-api/wham/apps`
- browser task pages via `/codex/tasks/<task_id>`

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
- ChatGPT backend base URL: `http://127.0.0.1:8765/backend-api`
- models endpoints: `http://127.0.0.1:8765/models`, `http://127.0.0.1:8765/v1/models`, and `http://127.0.0.1:8765/backend-api/codex/models`
- responses endpoint: `http://127.0.0.1:8765/backend-api/codex/responses`
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

To exercise ChatGPT token refresh against the mock, point the existing override
at the mock token endpoint before starting Codex:

```bash
export CODEX_REFRESH_TOKEN_URL_OVERRIDE=http://127.0.0.1:8765/oauth/token
```

If the login server default issuer is the production value
`https://auth.openai.com`, also point ChatGPT login itself at the mock:

```bash
export CODEX_LOGIN_ISSUER_OVERRIDE=http://127.0.0.1:8765
```

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

Point `chatgpt_base_url` at the mock backend-api root. Example:

```toml
chatgpt_base_url = "http://127.0.0.1:8765/backend-api/"
```

There is a ready-to-copy example at [scripts/mock-chatgpt-account-config.toml.example](/D:/agentx/codex/codex-rs/scripts/mock-chatgpt-account-config.toml.example).

Recommended workflow for TUI or app-server:

1. Log in once with `codex login` against the mock issuer.
2. Set `chatgpt_base_url` to the mock backend-api root.
3. Point the OpenAI model provider at the mock ChatGPT models endpoint.
4. Optionally set `CODEX_REFRESH_TOKEN_URL_OVERRIDE` to the mock `/oauth/token`.
5. Start TUI or app-server normally.
6. Call `account/read`, `account/rateLimits/read`, or run normal turns.

The mock `/backend-api/wham/usage` returns:

- a primary window
- a secondary window
- any repeated `--additional-limit` buckets

To point Codex's ChatGPT-mode model discovery at the mock, set the OpenAI
provider base URL to the mock ChatGPT models root, for example:

```toml
[model_providers.openai]
base_url = "http://127.0.0.1:8765/backend-api/codex"
```

That combination covers the ChatGPT-mode defaults:

- Responses: `/backend-api/codex/responses`
- Models: `/backend-api/codex/models`
- Rate limits and tasks: `/backend-api/wham/*`
- Browser task links: `/codex/tasks/<task_id>`
- MCP apps: `/backend-api/wham/apps`

## App-Server ChatGPT Login

`account/login/start(type=chatgpt)` can now use the local mock issuer as long
as `CODEX_LOGIN_ISSUER_OVERRIDE` is set before the app-server process starts.
