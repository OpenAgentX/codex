#!/usr/bin/env python3
"""Local mock server for Codex ChatGPT account debugging.

This server provides the smallest HTTP surface needed to debug the Codex
ChatGPT account flow without talking to real OpenAI services.

Supported flows:
- Browser login via `/oauth/authorize` -> local Codex callback.
- Device code login via `/api/accounts/deviceauth/*` and `/codex/device`.
- Token exchange via `/oauth/token`.
- Rate limits via `/api/codex/usage`.

The implementation uses only the Python standard library.
"""

from __future__ import annotations

import argparse
import base64
import html
import json
import secrets
import threading
import time
from http.cookies import SimpleCookie
from dataclasses import dataclass, field
from http import HTTPStatus
from http.server import BaseHTTPRequestHandler
from http.server import ThreadingHTTPServer
from typing import Dict
from typing import Optional
from urllib.parse import parse_qs
from urllib.parse import urlencode
from urllib.parse import urlparse


def b64url(data: bytes) -> str:
    return base64.urlsafe_b64encode(data).rstrip(b"=").decode("ascii")


def fake_jwt(payload: dict) -> str:
    header = {"alg": "none", "typ": "JWT"}
    return ".".join(
        (
            b64url(json.dumps(header, separators=(",", ":")).encode("utf-8")),
            b64url(json.dumps(payload, separators=(",", ":")).encode("utf-8")),
            b64url(b"signature"),
        )
    )


def unix_now() -> int:
    return int(time.time())


@dataclass
class LimitBucket:
    limit_id: str
    used_percent: int
    window_mins: int
    resets_in_secs: int
    limit_name: Optional[str] = None

    @classmethod
    def parse(cls, raw: str) -> "LimitBucket":
        parts = raw.split(":")
        if len(parts) not in (4, 5):
            raise argparse.ArgumentTypeError(
                "additional limit must be LIMIT_ID:USED_PERCENT:WINDOW_MINS:RESETS_IN_SECS[:LIMIT_NAME]"
            )
        limit_id, used, window_mins, resets_in_secs = parts[:4]
        limit_name = parts[4] if len(parts) == 5 else None
        try:
            return cls(
                limit_id=limit_id,
                used_percent=int(used),
                window_mins=int(window_mins),
                resets_in_secs=int(resets_in_secs),
                limit_name=limit_name,
            )
        except ValueError as exc:
            raise argparse.ArgumentTypeError(
                "additional limit numeric fields must be integers"
            ) from exc

    def as_usage_payload(self) -> dict:
        return {
            "limit_name": self.limit_name or self.limit_id,
            "metered_feature": self.limit_id,
            "rate_limit": {
                "allowed": True,
                "limit_reached": self.used_percent >= 100,
                "primary_window": {
                    "used_percent": self.used_percent,
                    "limit_window_seconds": self.window_mins * 60,
                    "reset_after_seconds": self.resets_in_secs,
                    "reset_at": unix_now() + self.resets_in_secs,
                },
            },
        }


@dataclass
class DeviceCodeRecord:
    device_auth_id: str
    user_code: str
    approved: bool = False
    polls: int = 0
    authorization_code: str = field(
        default_factory=lambda: f"device-code-{secrets.token_urlsafe(10)}"
    )
    code_challenge: str = field(
        default_factory=lambda: f"challenge-{secrets.token_urlsafe(8)}"
    )
    code_verifier: str = field(
        default_factory=lambda: f"verifier-{secrets.token_urlsafe(16)}"
    )


@dataclass
class ServerState:
    args: argparse.Namespace
    device_codes: Dict[str, DeviceCodeRecord] = field(default_factory=dict)
    auth_codes: Dict[str, float] = field(default_factory=dict)
    browser_sessions: Dict[str, str] = field(default_factory=dict)
    lock: threading.Lock = field(default_factory=threading.Lock)

    def build_auth_claims(self) -> dict:
        return {
            "chatgpt_plan_type": self.args.plan_type,
            "chatgpt_user_id": self.args.chatgpt_user_id,
            "chatgpt_account_id": self.args.chatgpt_account_id,
            "organization_id": self.args.organization_id
            or self.args.chatgpt_account_id,
            "project_id": self.args.project_id,
            "completed_platform_onboarding": self.args.completed_platform_onboarding,
            "is_org_owner": self.args.is_org_owner,
        }

    def build_id_token(self) -> str:
        payload = {
            "email": self.args.email,
            "https://api.openai.com/profile": {
                "email": self.args.email,
            },
            "https://api.openai.com/auth": self.build_auth_claims(),
        }
        return fake_jwt(payload)

    def build_access_token(self) -> str:
        payload = {
            "sub": self.args.chatgpt_user_id,
            "jti": self.args.access_token,
            "https://api.openai.com/auth": self.build_auth_claims(),
        }
        return fake_jwt(payload)

    def build_models_response(self) -> dict:
        return {
            "models": [
                {
                    "slug": self.args.model_slug,
                    "display_name": self.args.model_display_name,
                    "description": self.args.model_description,
                    "default_reasoning_level": self.args.model_default_reasoning_level,
                    "supported_reasoning_levels": [
                        {
                            "effort": "low",
                            "description": "Fast responses with lighter reasoning",
                        },
                        {
                            "effort": "medium",
                            "description": "Balances speed and reasoning depth for everyday tasks",
                        },
                        {
                            "effort": "high",
                            "description": "Greater reasoning depth for complex problems",
                        },
                        {
                            "effort": "xhigh",
                            "description": "Extra high reasoning depth for complex problems",
                        },
                    ],
                    "shell_type": "shell_command",
                    "visibility": "list",
                    "supported_in_api": True,
                    "priority": self.args.model_priority,
                    "availability_nux": None,
                    "upgrade": None,
                    "base_instructions": (
                        "You are Codex, a coding agent running against the local "
                        "mock ChatGPT account server."
                    ),
                    "supports_reasoning_summaries": True,
                    "default_reasoning_summary": "auto",
                    "support_verbosity": True,
                    "default_verbosity": "low",
                    "apply_patch_tool_type": "freeform",
                    "web_search_tool_type": "text",
                    "truncation_policy": {
                        "mode": "tokens",
                        "limit": self.args.model_truncation_limit,
                    },
                    "supports_parallel_tool_calls": True,
                    "supports_image_detail_original": True,
                    "context_window": self.args.model_context_window,
                    "experimental_supported_tools": [],
                    "input_modalities": ["text", "image"],
                    "prefer_websockets": False,
                    "supports_search_tool": False,
                }
            ]
        }

    def create_auth_code(self, prefix: str) -> str:
        code = f"{prefix}-{secrets.token_urlsafe(12)}"
        with self.lock:
            self.auth_codes[code] = time.time()
        return code

    def create_browser_session(self, username: str) -> str:
        session_id = f"session-{secrets.token_urlsafe(18)}"
        with self.lock:
            self.browser_sessions[session_id] = username
        return session_id

    def has_browser_session(self, session_id: Optional[str]) -> bool:
        if not session_id:
            return False
        with self.lock:
            return session_id in self.browser_sessions

    def clear_browser_session(self, session_id: Optional[str]) -> None:
        if not session_id:
            return
        with self.lock:
            self.browser_sessions.pop(session_id, None)

    def mark_device_code_approved(self, user_code: str) -> bool:
        normalized = user_code.strip().upper()
        with self.lock:
            for record in self.device_codes.values():
                if record.user_code == normalized:
                    record.approved = True
                    return True
        return False

    def find_device_code(self, device_auth_id: str, user_code: str) -> Optional[DeviceCodeRecord]:
        normalized = user_code.strip().upper()
        with self.lock:
            record = self.device_codes.get(device_auth_id)
            if record is None or record.user_code != normalized:
                return None
            return record


class MockHandler(BaseHTTPRequestHandler):
    server: "MockServer"
    BROWSER_SESSION_COOKIE = "mock_codex_browser_session"

    def do_GET(self) -> None:
        parsed = urlparse(self.path)
        if parsed.path == "/healthz":
            self.respond_json({"ok": True})
            return
        if parsed.path in {"/models", "/v1/models"}:
            self.handle_models()
            return
        if parsed.path == "/oauth/authorize":
            self.handle_authorize(parsed)
            return
        if parsed.path == "/oauth/logout":
            self.handle_browser_logout(parsed)
            return
        if parsed.path == "/codex/device":
            self.render_device_page(parsed, message=None)
            return
        if parsed.path == "/api/codex/usage":
            self.handle_usage()
            return
        if parsed.path == "/deviceauth/callback":
            self.respond_html(
                HTTPStatus.OK,
                "<html><body><h1>Mock device callback reached</h1></body></html>",
            )
            return
        self.respond_json(
            {"error": f"unknown path: {parsed.path}"},
            status=HTTPStatus.NOT_FOUND,
        )

    def do_POST(self) -> None:
        parsed = urlparse(self.path)
        if parsed.path == "/oauth/login":
            self.handle_browser_login()
            return
        if parsed.path == "/oauth/token":
            self.handle_oauth_token()
            return
        if parsed.path == "/api/accounts/deviceauth/usercode":
            self.handle_device_usercode()
            return
        if parsed.path == "/api/accounts/deviceauth/token":
            self.handle_device_token()
            return
        if parsed.path == "/codex/device":
            self.handle_device_approval()
            return
        self.respond_json(
            {"error": f"unknown path: {parsed.path}"},
            status=HTTPStatus.NOT_FOUND,
        )

    def log_message(self, format: str, *args: object) -> None:
        print(f"[mock-account] {self.address_string()} - {format % args}")

    def read_body(self) -> bytes:
        length = int(self.headers.get("Content-Length", "0"))
        return self.rfile.read(length)

    def send_body(
        self,
        body: bytes,
        content_type: str,
        status: HTTPStatus = HTTPStatus.OK,
        extra_headers: Optional[list[tuple[str, str]]] = None,
    ) -> None:
        self.send_response(status.value)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body)))
        for key, value in extra_headers or []:
            self.send_header(key, value)
        self.end_headers()
        self.wfile.write(body)

    def respond_json(
        self,
        payload: dict,
        status: HTTPStatus = HTTPStatus.OK,
        extra_headers: Optional[list[tuple[str, str]]] = None,
    ) -> None:
        body = json.dumps(payload, separators=(",", ":")).encode("utf-8")
        self.send_body(
            body,
            "application/json",
            status=status,
            extra_headers=extra_headers,
        )

    def respond_html(
        self,
        status: HTTPStatus,
        html_body: str,
        extra_headers: Optional[list[tuple[str, str]]] = None,
    ) -> None:
        body = html_body.encode("utf-8")
        self.send_body(
            body,
            "text/html; charset=utf-8",
            status=status,
            extra_headers=extra_headers,
        )

    def redirect(
        self,
        location: str,
        extra_headers: Optional[list[tuple[str, str]]] = None,
    ) -> None:
        self.send_response(HTTPStatus.FOUND.value)
        self.send_header("Location", location)
        self.send_header("Content-Length", "0")
        for key, value in extra_headers or []:
            self.send_header(key, value)
        self.end_headers()

    def parse_form_body(self) -> dict[str, list[str]]:
        return parse_qs(self.read_body().decode("utf-8"))

    def browser_login_username(self) -> str:
        return self.server.state.args.login_username or self.server.state.args.email

    def browser_login_password(self) -> str:
        return self.server.state.args.login_password

    def browser_session_id(self) -> Optional[str]:
        raw_cookie = self.headers.get("Cookie")
        if not raw_cookie:
            return None
        cookie = SimpleCookie()
        cookie.load(raw_cookie)
        morsel = cookie.get(self.BROWSER_SESSION_COOKIE)
        if morsel is None:
            return None
        return morsel.value

    def is_browser_authenticated(self) -> bool:
        return self.server.state.has_browser_session(self.browser_session_id())

    @staticmethod
    def original_authorize_path(parsed) -> str:
        return parsed.path if not parsed.query else f"{parsed.path}?{parsed.query}"

    @staticmethod
    def normalize_continue_to(continue_to: str) -> str:
        if continue_to.startswith("/oauth/authorize"):
            return continue_to
        return "/oauth/authorize"

    @staticmethod
    def normalize_logout_continue_to(continue_to: str) -> str:
        if continue_to.startswith("/"):
            return continue_to
        return "/oauth/authorize"

    def handle_authorize(self, parsed) -> None:
        params = parse_qs(parsed.query)
        redirect_uri = params.get("redirect_uri", [None])[0]
        state = params.get("state", [""])[0]
        if not redirect_uri:
            self.respond_json(
                {"error": "redirect_uri is required"},
                status=HTTPStatus.BAD_REQUEST,
            )
            return
        if not self.is_browser_authenticated():
            self.render_browser_login_page(
                continue_to=self.original_authorize_path(parsed),
                error_message=None,
            )
            return
        code = self.server.state.create_auth_code("auth")
        query = urlencode({"code": code, "state": state})
        separator = "&" if "?" in redirect_uri else "?"
        self.redirect(f"{redirect_uri}{separator}{query}")

    def handle_browser_login(self) -> None:
        params = self.parse_form_body()
        username = params.get("username", [""])[0]
        password = params.get("password", [""])[0]
        continue_to = self.normalize_continue_to(params.get("continue_to", [""])[0])
        if (
            username != self.browser_login_username()
            or password != self.browser_login_password()
        ):
            self.render_browser_login_page(
                continue_to=continue_to,
                error_message="Invalid username or password.",
            )
            return

        session_id = self.server.state.create_browser_session(username)
        self.redirect(
            continue_to,
            extra_headers=[
                (
                    "Set-Cookie",
                    (
                        f"{self.BROWSER_SESSION_COOKIE}={session_id}; "
                        "HttpOnly; Path=/; SameSite=Lax"
                    ),
                )
            ],
        )

    def handle_browser_logout(self, parsed) -> None:
        params = parse_qs(parsed.query)
        continue_to = self.normalize_logout_continue_to(
            params.get("continue_to", ["/oauth/authorize"])[0]
        )
        self.server.state.clear_browser_session(self.browser_session_id())
        self.redirect(
            continue_to,
            extra_headers=[
                (
                    "Set-Cookie",
                    (
                        f"{self.BROWSER_SESSION_COOKIE}=; "
                        "Expires=Thu, 01 Jan 1970 00:00:00 GMT; "
                        "HttpOnly; Path=/; SameSite=Lax"
                    ),
                )
            ],
        )

    def handle_oauth_token(self) -> None:
        content_type = self.headers.get("Content-Type", "")
        if "application/x-www-form-urlencoded" not in content_type:
            self.respond_json(
                {"error": "expected application/x-www-form-urlencoded"},
                status=HTTPStatus.BAD_REQUEST,
            )
            return
        body = self.read_body().decode("utf-8")
        params = parse_qs(body)
        grant_type = params.get("grant_type", [""])[0]
        if grant_type == "authorization_code":
            self.respond_json(
                {
                    "id_token": self.server.state.build_id_token(),
                    "access_token": self.server.state.build_access_token(),
                    "refresh_token": self.server.state.args.refresh_token,
                }
            )
            return
        if grant_type == "urn:ietf:params:oauth:grant-type:token-exchange":
            self.respond_json({"access_token": self.server.state.args.api_key})
            return
        self.respond_json(
            {"error": f"unsupported grant_type: {grant_type}"},
            status=HTTPStatus.BAD_REQUEST,
        )

    def handle_models(self) -> None:
        self.respond_json(
            self.server.state.build_models_response(),
            extra_headers=[("ETag", self.server.state.args.models_etag)],
        )

    def handle_device_usercode(self) -> None:
        record = DeviceCodeRecord(
            device_auth_id=f"device-auth-{secrets.token_urlsafe(8)}",
            user_code=self.generate_user_code(),
        )
        with self.server.state.lock:
            self.server.state.device_codes[record.device_auth_id] = record
        self.respond_json(
            {
                "device_auth_id": record.device_auth_id,
                "user_code": record.user_code,
                "interval": str(self.server.state.args.device_code_interval_secs),
            }
        )

    def handle_device_token(self) -> None:
        try:
            payload = json.loads(self.read_body().decode("utf-8"))
        except json.JSONDecodeError:
            self.respond_json(
                {"error": "invalid json body"},
                status=HTTPStatus.BAD_REQUEST,
            )
            return
        device_auth_id = payload.get("device_auth_id", "")
        user_code = payload.get("user_code", "")
        record = self.server.state.find_device_code(device_auth_id, user_code)
        if record is None:
            self.respond_json(
                {"error": "unknown device code"},
                status=HTTPStatus.NOT_FOUND,
            )
            return

        with self.server.state.lock:
            record.polls += 1
            approved = record.approved or (
                self.server.state.args.device_code_auto_approve
                and record.polls > self.server.state.args.device_code_pending_polls
            )
            if approved:
                record.approved = True
                response = {
                    "authorization_code": record.authorization_code,
                    "code_challenge": record.code_challenge,
                    "code_verifier": record.code_verifier,
                }
            else:
                response = None

        if response is None:
            self.respond_json(
                {"status": "pending"},
                status=HTTPStatus.NOT_FOUND,
            )
            return
        self.respond_json(response)

    def handle_usage(self) -> None:
        auth_header = self.headers.get("authorization")
        account_id = self.headers.get("chatgpt-account-id")
        if auth_header is None or not auth_header.startswith("Bearer "):
            self.respond_json(
                {"error": "missing bearer token"},
                status=HTTPStatus.UNAUTHORIZED,
            )
            return
        if self.server.state.args.strict_account_header and account_id != self.server.state.args.chatgpt_account_id:
            self.respond_json(
                {"error": "chatgpt-account-id mismatch"},
                status=HTTPStatus.UNAUTHORIZED,
            )
            return

        body = {
            "plan_type": self.server.state.args.plan_type,
            "rate_limit": {
                "allowed": True,
                "limit_reached": self.server.state.args.primary_used_percent >= 100,
                "primary_window": {
                    "used_percent": self.server.state.args.primary_used_percent,
                    "limit_window_seconds": self.server.state.args.primary_window_mins * 60,
                    "reset_after_seconds": self.server.state.args.primary_resets_in_secs,
                    "reset_at": unix_now() + self.server.state.args.primary_resets_in_secs,
                },
                "secondary_window": None,
            },
            "additional_rate_limits": [
                bucket.as_usage_payload()
                for bucket in self.server.state.args.additional_limit
            ],
        }
        if self.server.state.args.secondary_used_percent is not None:
            body["rate_limit"]["secondary_window"] = {
                "used_percent": self.server.state.args.secondary_used_percent,
                "limit_window_seconds": self.server.state.args.secondary_window_mins * 60,
                "reset_after_seconds": self.server.state.args.secondary_resets_in_secs,
                "reset_at": unix_now() + self.server.state.args.secondary_resets_in_secs,
            }
        self.respond_json(body)

    def handle_device_approval(self) -> None:
        params = self.parse_form_body()
        user_code = params.get("user_code", [""])[0]
        message = "Approved device code." if self.server.state.mark_device_code_approved(user_code) else "Device code not found."
        self.render_device_page(urlparse(self.path), message=message)

    def render_browser_login_page(
        self, continue_to: str, error_message: Optional[str]
    ) -> None:
        username = html.escape(self.browser_login_username())
        password_hint = html.escape(self.browser_login_password())
        continue_to = html.escape(continue_to, quote=True)
        error_html = (
            f"<p class='error'>{html.escape(error_message)}</p>"
            if error_message
            else ""
        )
        body = f"""<!doctype html>
<html>
<head>
  <meta charset="utf-8">
  <title>Mock Codex Browser Login</title>
  <style>
    body {{ font-family: sans-serif; margin: 0; background: #f5f7fb; color: #111827; }}
    .shell {{ max-width: 420px; margin: 8vh auto; background: white; border: 1px solid #d1d5db; border-radius: 16px; padding: 2rem; box-shadow: 0 12px 30px rgba(0, 0, 0, 0.08); }}
    h1 {{ margin: 0 0 0.75rem; font-size: 1.5rem; }}
    p {{ line-height: 1.5; }}
    .hint {{ color: #4b5563; font-size: 0.95rem; }}
    .error {{ color: #b91c1c; background: #fef2f2; border: 1px solid #fecaca; padding: 0.75rem; border-radius: 10px; }}
    form {{ display: grid; gap: 0.85rem; margin-top: 1.25rem; }}
    label {{ display: grid; gap: 0.35rem; font-weight: 600; }}
    input {{ padding: 0.7rem 0.8rem; border: 1px solid #cbd5e1; border-radius: 10px; font: inherit; }}
    button {{ margin-top: 0.25rem; padding: 0.8rem 1rem; border: 0; border-radius: 999px; background: #111827; color: white; font: inherit; cursor: pointer; }}
    code {{ background: #f3f4f6; padding: 0.1rem 0.35rem; border-radius: 6px; }}
  </style>
</head>
<body>
  <div class="shell">
    <h1>Sign in to Mock Codex</h1>
    <p class="hint">This mock issuer now requires a browser login before it will complete <code>/oauth/authorize</code>.</p>
    <p class="hint">Configured credentials: <code>{username}</code> / <code>{password_hint}</code></p>
    {error_html}
    <form method="post" action="/oauth/login">
      <input type="hidden" name="continue_to" value="{continue_to}" />
      <label>
        Username
        <input name="username" autocomplete="username" />
      </label>
      <label>
        Password
        <input name="password" type="password" autocomplete="current-password" />
      </label>
      <button type="submit">Continue</button>
    </form>
  </div>
</body>
</html>"""
        self.respond_html(HTTPStatus.OK, body)

    def render_device_page(self, parsed, message: Optional[str]) -> None:
        with self.server.state.lock:
            records = list(self.server.state.device_codes.values())
        rows = []
        for record in records:
            rows.append(
                "<tr>"
                f"<td><code>{html.escape(record.user_code)}</code></td>"
                f"<td>{'yes' if record.approved else 'no'}</td>"
                f"<td>{record.polls}</td>"
                "</tr>"
            )
        table_rows = "".join(rows) or "<tr><td colspan='3'>No active device codes yet.</td></tr>"
        flash = f"<p><strong>{html.escape(message)}</strong></p>" if message else ""
        body = f"""<!doctype html>
<html>
<head>
  <meta charset="utf-8">
  <title>Mock Codex Device Auth</title>
  <style>
    body {{ font-family: sans-serif; margin: 2rem auto; max-width: 48rem; }}
    code {{ background: #f4f4f4; padding: 0.2rem 0.35rem; }}
    table {{ border-collapse: collapse; width: 100%; margin-top: 1rem; }}
    td, th {{ border: 1px solid #ddd; padding: 0.5rem; text-align: left; }}
    form {{ display: flex; gap: 0.5rem; margin-top: 1rem; }}
    input {{ flex: 1; padding: 0.5rem; }}
    button {{ padding: 0.5rem 0.8rem; }}
  </style>
</head>
<body>
  <h1>Mock Codex Device Auth</h1>
  <p>Open this page after <code>codex login --device-auth</code> prints a user code.</p>
  {flash}
  <form method="post" action="/codex/device">
    <input name="user_code" placeholder="Enter the printed user code" />
    <button type="submit">Approve</button>
  </form>
  <table>
    <thead>
      <tr><th>User code</th><th>Approved</th><th>Polls</th></tr>
    </thead>
    <tbody>{table_rows}</tbody>
  </table>
</body>
</html>"""
        self.respond_html(HTTPStatus.OK, body)

    @staticmethod
    def generate_user_code() -> str:
        return f"{secrets.token_hex(2).upper()}-{secrets.token_hex(2).upper()}"


class MockServer(ThreadingHTTPServer):
    def __init__(self, server_address, handler_cls, state: ServerState) -> None:
        super().__init__(server_address, handler_cls)
        self.state = state


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Local mock server for Codex ChatGPT account flows."
    )
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=8765)
    parser.add_argument("--email", default="debug@example.com")
    parser.add_argument(
        "--login-username",
        help="Browser login username. Defaults to --email when omitted.",
    )
    parser.add_argument(
        "--login-password",
        default="debug-password",
        help="Browser login password used by the mock /oauth/authorize flow.",
    )
    parser.add_argument("--plan-type", default="pro")
    parser.add_argument("--chatgpt-account-id", default="org-debug")
    parser.add_argument("--chatgpt-user-id", default="user-debug")
    parser.add_argument("--organization-id")
    parser.add_argument("--project-id", default="")
    parser.add_argument(
        "--completed-platform-onboarding",
        action="store_true",
        default=True,
        help="Emit completed_platform_onboarding=true in the mock JWT auth claims.",
    )
    parser.add_argument(
        "--no-completed-platform-onboarding",
        dest="completed_platform_onboarding",
        action="store_false",
        help="Emit completed_platform_onboarding=false in the mock JWT auth claims.",
    )
    parser.add_argument(
        "--is-org-owner",
        action="store_true",
        help="Emit is_org_owner=true in the mock JWT auth claims.",
    )
    parser.add_argument(
        "--access-token",
        default="mock-chatgpt-access-token",
        help="Opaque identifier stored in the mock JWT access token jti claim.",
    )
    parser.add_argument("--refresh-token", default="mock-chatgpt-refresh-token")
    parser.add_argument("--api-key", default="sk-mock-api-key")
    parser.add_argument("--model-slug", default="gpt-5.3-codex")
    parser.add_argument("--model-display-name", default="gpt-5.3-codex")
    parser.add_argument(
        "--model-description",
        default="Mock remote model served by the local ChatGPT account server.",
    )
    parser.add_argument("--model-default-reasoning-level", default="medium")
    parser.add_argument("--model-priority", type=int, default=0)
    parser.add_argument("--model-context-window", type=int, default=272000)
    parser.add_argument("--model-truncation-limit", type=int, default=10000)
    parser.add_argument("--models-etag", default="mock-models-etag-v1")
    parser.add_argument("--primary-used-percent", type=int, default=42)
    parser.add_argument("--primary-window-mins", type=int, default=60)
    parser.add_argument("--primary-resets-in-secs", type=int, default=120)
    parser.add_argument("--secondary-used-percent", type=int, default=5)
    parser.add_argument("--secondary-window-mins", type=int, default=1440)
    parser.add_argument("--secondary-resets-in-secs", type=int, default=43200)
    parser.add_argument(
        "--additional-limit",
        action="append",
        type=LimitBucket.parse,
        default=[],
        help="LIMIT_ID:USED_PERCENT:WINDOW_MINS:RESETS_IN_SECS[:LIMIT_NAME]",
    )
    parser.add_argument("--device-code-interval-secs", type=int, default=1)
    parser.add_argument("--device-code-pending-polls", type=int, default=1)
    parser.add_argument(
        "--device-code-auto-approve",
        action="store_true",
        help="Automatically approve device codes after the pending poll threshold.",
    )
    parser.add_argument(
        "--strict-account-header",
        action="store_true",
        help="Require `chatgpt-account-id` to match the configured account id on /api/codex/usage.",
    )
    return parser


def main() -> int:
    parser = build_parser()
    args = parser.parse_args()
    state = ServerState(args=args)
    server = MockServer((args.host, args.port), MockHandler, state)
    login_username = args.login_username or args.email

    print(f"Mock account server listening on http://{args.host}:{args.port}")
    print(f"OAuth issuer: http://{args.host}:{args.port}")
    print(f"Browser login: {login_username} / {args.login_password}")
    print(f"Models endpoints: http://{args.host}:{args.port}/models and /v1/models")
    print(f"Rate limit base URL: http://{args.host}:{args.port}")
    print(f"Device auth page: http://{args.host}:{args.port}/codex/device")

    try:
        server.serve_forever()
    except KeyboardInterrupt:
        print("\nShutting down mock account server.")
    finally:
        server.server_close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
