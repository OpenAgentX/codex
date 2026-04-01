# Mock ChatGPT Account Server

This crate provides a standalone Rust implementation of `scripts/mock_chatgpt_account_server.py`.

It is intentionally isolated from the existing `codex-*` library crates. The server uses only direct
third-party dependencies plus an embedded React + TypeScript single-page app for the browser login,
device authorization, and task detail pages.

## Run

```bash
cargo run -p codex-mock-chatgpt-account-server --bin mock_chatgpt_account_server
```

The default server listens on `127.0.0.1:8765`.

To enable Google and GitHub OAuth login buttons on the browser sign-in page, point the server at
the bundled example TOML file:

```bash
cargo run -p codex-mock-chatgpt-account-server --bin mock_chatgpt_account_server -- \
  --social-login-config mock-chatgpt-account-server/social-login.example.toml
```

Each enabled provider now acts as a real OAuth client. You must register these callback URLs in the
Google or GitHub developer console before the local mock server can complete login:

```text
http://127.0.0.1:8765/oauth/login/google/callback
http://127.0.0.1:8765/oauth/login/github/callback
```

The TOML format is:

```toml
[social_login.google]
enabled = true
label = "Continue with Google"
subtitle = "debug@example.com"
client_id = "replace-with-google-client-id"
client_secret = "replace-with-google-client-secret"
authorize_url = "https://accounts.google.com/o/oauth2/v2/auth"
token_url = "https://oauth2.googleapis.com/token"
user_info_url = "https://openidconnect.googleapis.com/v1/userinfo"
scopes = ["openid", "email", "profile"]

[social_login.github]
enabled = true
label = "Continue with GitHub"
subtitle = "@debug-codex"
client_id = "replace-with-github-client-id"
client_secret = "replace-with-github-client-secret"
authorize_url = "https://github.com/login/oauth/authorize"
token_url = "https://github.com/login/oauth/access_token"
user_info_url = "https://api.github.com/user"
user_email_url = "https://api.github.com/user/emails"
scopes = ["read:user", "user:email"]
```

## Frontend

The SPA source lives under [`frontend/`](./frontend). The checked-in `dist/` assets are embedded into
the Rust binary with `include_str!`. Styling is authored with Tailwind CSS and PostCSS.

To rebuild the frontend bundle after editing the React or TypeScript source:

```bash
cd mock-chatgpt-account-server/frontend
npm install
npm run build
```
