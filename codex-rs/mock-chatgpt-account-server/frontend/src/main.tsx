import { FormEvent, ReactNode, StrictMode, useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import "./styles.css";

type LoginBootstrap = {
  page: "browserLogin";
  continueTo: string;
  usernameHint: string;
  passwordHint: string;
  errorMessage: string | null;
  socialProviders: SocialLoginProvider[];
};

type DeviceRecord = {
  userCode: string;
  approved: boolean;
  polls: number;
};

type SocialLoginProvider = {
  id: string;
  label: string;
  subtitle: string | null;
};

type DeviceBootstrap = {
  page: "deviceAuth";
  records: DeviceRecord[];
  message: string | null;
};

type AccountConfirmBootstrap = {
  page: "accountConfirm";
  continueTo: string;
  email: string;
  accountId: string;
  planType: string;
  organizationId: string;
  projectId: string;
  redirectUri: string;
  oauthState: string;
};

type TaskData = {
  taskId: string;
  title: string;
  userPrompt: string;
  assistantResponse: string;
};

type TaskBootstrap = {
  page: "taskView";
  task: TaskData | null;
  missingTaskId: string | null;
};

type CallbackBootstrap = {
  page: "callback";
};

type Bootstrap =
  | LoginBootstrap
  | AccountConfirmBootstrap
  | DeviceBootstrap
  | TaskBootstrap
  | CallbackBootstrap;

type LoginActionResponse = {
  ok: boolean;
  redirectTo?: string;
  errorMessage?: string;
};

type LoginStep = "email" | "password";

function readBootstrap(): Bootstrap {
  const script = document.getElementById("mock-bootstrap");
  if (!script?.textContent) {
    throw new Error("missing bootstrap payload");
  }
  return JSON.parse(script.textContent) as Bootstrap;
}

function Shell(props: { title: string; eyebrow: string; children: ReactNode }) {
  return (
    <main className="page-shell">
      <section className="hero-panel">
        <p className="eyebrow">{props.eyebrow}</p>
        <h1>{props.title}</h1>
        {props.children}
      </section>
    </main>
  );
}

function AuthFooterLinks() {
  function preventDefault(event: React.MouseEvent<HTMLAnchorElement>) {
    event.preventDefault();
  }

  return (
    <div className="auth-footer-links">
      <a href="#" onClick={preventDefault}>
        使用条款
      </a>
      <span>|</span>
      <a href="#" onClick={preventDefault}>
        隐私政策
      </a>
    </div>
  );
}

function AuthGlyph() {
  return (
    <div className="auth-glyph">
      <svg aria-hidden="true" fill="none" viewBox="0 0 40 40">
        <circle cx="20" cy="20" r="12" stroke="currentColor" strokeWidth="2.6" />
        <path d="M14 24h8" stroke="currentColor" strokeLinecap="round" strokeWidth="2.6" />
        <path d="M18 14a5 5 0 0 1 6 6" stroke="currentColor" strokeLinecap="round" strokeWidth="2.6" />
        <path d="M12.5 18.5 10.8 20l1.7 1.5" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" strokeWidth="2.6" />
      </svg>
    </div>
  );
}

function providerDisplayName(provider: SocialLoginProvider) {
  if (provider.id === "google") {
    return "Google";
  }
  if (provider.id === "github") {
    return "GitHub";
  }
  return provider.label;
}

function ProviderIcon(props: { providerId: string }) {
  if (props.providerId === "google") {
    return <span className="provider-icon provider-icon-google">G</span>;
  }
  if (props.providerId === "github") {
    return <span className="provider-icon provider-icon-github">GH</span>;
  }
  return <span className="provider-icon">{props.providerId.slice(0, 2).toUpperCase()}</span>;
}

function BrowserLoginPage(props: { bootstrap: LoginBootstrap }) {
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [errorMessage, setErrorMessage] = useState(props.bootstrap.errorMessage);
  const [submitting, setSubmitting] = useState(false);
  const [shortcutSubmitting, setShortcutSubmitting] = useState<string | null>(null);
  const [step, setStep] = useState<LoginStep>("email");

  useEffect(() => {
    document.title = "欢迎回来";
  }, []);

  function handleEmailContinue(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!email.trim()) {
      setErrorMessage("请输入电子邮件地址。");
      return;
    }
    setErrorMessage(null);
    setStep("password");
  }

  async function handlePasswordSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!password) {
      setErrorMessage("请输入密码。");
      return;
    }
    setSubmitting(true);
    setErrorMessage(null);

    const response = await fetch("/oauth/login", {
      method: "POST",
      headers: {
        Accept: "application/json",
        "Content-Type": "application/x-www-form-urlencoded",
      },
      body: new URLSearchParams({
        username: email.trim(),
        password,
        continue_to: props.bootstrap.continueTo,
      }).toString(),
      credentials: "same-origin",
    });
    const payload = (await response.json()) as LoginActionResponse;
    setSubmitting(false);

    if (!payload.ok) {
      setErrorMessage(payload.errorMessage ?? "登录失败，请检查邮箱和密码。");
      return;
    }

    window.location.assign(payload.redirectTo ?? props.bootstrap.continueTo);
  }

  async function handleShortcutSubmit(event: FormEvent<HTMLFormElement>, providerId: string) {
    event.preventDefault();
    setShortcutSubmitting(providerId);
    setErrorMessage(null);

    const response = await fetch("/oauth/login/shortcut", {
      method: "POST",
      headers: {
        Accept: "application/json",
        "Content-Type": "application/x-www-form-urlencoded",
      },
      body: new URLSearchParams({
        provider: providerId,
        continue_to: props.bootstrap.continueTo,
      }).toString(),
      credentials: "same-origin",
    });
    const payload = (await response.json()) as LoginActionResponse;
    setShortcutSubmitting(null);

    if (!payload.ok) {
      setErrorMessage(payload.errorMessage ?? "快捷登录失败。");
      return;
    }

    window.location.assign(payload.redirectTo ?? props.bootstrap.continueTo);
  }

  return (
    <main className="auth-page">
      <section className="login-shell">
        <h1 className="login-title">欢迎回来</h1>
        {errorMessage ? <p className="auth-error">{errorMessage}</p> : null}
        {step === "email" ? (
          <form action="/oauth/login" className="login-form" method="post" onSubmit={handleEmailContinue}>
            <input
              autoComplete="username"
              className="auth-input"
              name="username"
              placeholder="电子邮件地址"
              value={email}
              onChange={(event) => setEmail(event.target.value)}
            />
            <button className="auth-primary-button" disabled={shortcutSubmitting !== null} type="submit">
              继续
            </button>
          </form>
        ) : (
          <form action="/oauth/login" className="login-form" method="post" onSubmit={handlePasswordSubmit}>
            <div className="identity-pill-row">
              <button
                className="identity-pill"
                type="button"
                onClick={() => {
                  setStep("email");
                  setPassword("");
                  setErrorMessage(null);
                }}
              >
                <span className="identity-pill-icon">@</span>
                <span>{email.trim() || props.bootstrap.usernameHint}</span>
              </button>
            </div>
            <input name="continue_to" type="hidden" value={props.bootstrap.continueTo} />
            <input name="username" type="hidden" value={email.trim()} />
            <input
              autoComplete="current-password"
              className="auth-input"
              name="password"
              placeholder="密码"
              type="password"
              value={password}
              onChange={(event) => setPassword(event.target.value)}
            />
            <button className="auth-primary-button" disabled={submitting} type="submit">
              {submitting ? "继续中..." : "继续"}
            </button>
          </form>
        )}
        <p className="register-line">
          还没有账户？
          <a href="#" onClick={(event) => event.preventDefault()}>
            请注册
          </a>
        </p>
        {props.bootstrap.socialProviders.length > 0 ? (
          <>
            <div className="divider-line">
              <span>或</span>
            </div>
            <div className="provider-list">
              {props.bootstrap.socialProviders.map((provider) => (
                <form
                  action="/oauth/login/shortcut"
                  className="provider-form"
                  key={provider.id}
                  method="post"
                  onSubmit={(event) => handleShortcutSubmit(event, provider.id)}
                >
                  <input name="provider" type="hidden" value={provider.id} />
                  <input name="continue_to" type="hidden" value={props.bootstrap.continueTo} />
                  <button className="provider-button" disabled={shortcutSubmitting !== null} type="submit">
                    <span className="provider-button-main">
                      <ProviderIcon providerId={provider.id} />
                      <span>{`继续使用 ${providerDisplayName(provider)} 登录`}</span>
                    </span>
                    {shortcutSubmitting === provider.id ? <span className="provider-side-text">连接中</span> : null}
                  </button>
                </form>
              ))}
            </div>
          </>
        ) : null}
        <AuthFooterLinks />
      </section>
    </main>
  );
}

function AccountConfirmPage(props: { bootstrap: AccountConfirmBootstrap }) {
  const [submitting, setSubmitting] = useState(false);
  const workspaceOptions =
    props.bootstrap.organizationId && props.bootstrap.organizationId !== "personal"
      ? [
          {
            id: "workspace",
            name: props.bootstrap.organizationId,
            subtitle: `${props.bootstrap.planType.toUpperCase()} · 团队空间`,
            mark: props.bootstrap.organizationId.slice(0, 2).toLowerCase(),
            tone: "workspace" as const,
          },
          {
            id: "personal",
            name: "个人账户",
            subtitle: props.bootstrap.accountId,
            mark: props.bootstrap.email.slice(0, 1).toLowerCase(),
            tone: "personal" as const,
          },
        ]
      : [
          {
            id: "personal",
            name: "个人账户",
            subtitle: props.bootstrap.accountId,
            mark: props.bootstrap.email.slice(0, 1).toLowerCase(),
            tone: "personal" as const,
          },
        ];
  const [selectedWorkspace, setSelectedWorkspace] = useState(workspaceOptions[0]?.id ?? "personal");

  useEffect(() => {
    document.title = "使用 ChatGPT 登录到 Codex";
  }, []);

  async function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setSubmitting(true);

    const response = await fetch("/oauth/authorize/approve", {
      method: "POST",
      headers: {
        Accept: "application/json",
        "Content-Type": "application/x-www-form-urlencoded",
      },
      body: new URLSearchParams({
        continue_to: props.bootstrap.continueTo,
      }).toString(),
      credentials: "same-origin",
    });
    const payload = (await response.json()) as LoginActionResponse;
    setSubmitting(false);

    if (!payload.ok) {
      window.location.assign(props.bootstrap.continueTo);
      return;
    }

    window.location.assign(payload.redirectTo ?? props.bootstrap.redirectUri);
  }

  return (
    <main className="auth-page">
      <section className="confirm-shell">
        <AuthGlyph />
        <h1 className="confirm-title">使用 ChatGPT 登录到 Codex</h1>
        <div className="identity-pill-row">
          <div className="identity-pill static">
            <span className="identity-pill-icon">@</span>
            <span>{props.bootstrap.email}</span>
          </div>
        </div>
        <section className="workspace-section">
          <h2 className="workspace-heading">选择一个工作空间</h2>
          <div className="workspace-list">
            {workspaceOptions.map((workspace) => {
              const selected = selectedWorkspace === workspace.id;
              return (
                <button
                  className={`workspace-option ${selected ? "selected" : ""}`}
                  key={workspace.id}
                  type="button"
                  onClick={() => setSelectedWorkspace(workspace.id)}
                >
                  <span className={`workspace-mark ${workspace.tone}`}>
                    {workspace.mark}
                  </span>
                  <span className="workspace-copy">
                    <strong>{workspace.name}</strong>
                    <span>{workspace.subtitle}</span>
                  </span>
                  <span className={`workspace-check ${selected ? "visible" : ""}`}>✓</span>
                </button>
              );
            })}
          </div>
        </section>
        <div className="confirm-copy">
          <p>继续操作后，ChatGPT 将向 Codex 提供你的姓名、电子邮件和个人资料头像以关联你的帐户。</p>
          <p>Codex 不会收到你的聊天历史记录。</p>
          <p>
            在你使用 Codex 时：
            <br />
            该功能由你的 ChatGPT 帐户提供支持，并使用你当前套餐的速率限制、训练及语言偏好设置。
          </p>
          <p>
            ChatGPT 使用条款和隐私政策（或适用于 ChatGPT Enterprise、Education 或 Business 用户的对应服务条款）适用于与 ChatGPT
            共享的数据。
          </p>
          <p>Codex 可能存在错误。请务必审查其编写的代码和执行的命令。</p>
        </div>
        <div className="confirm-actions">
          <a
            className="auth-secondary-button"
            href={`/oauth/logout?continue_to=${encodeURIComponent(props.bootstrap.continueTo)}`}
          >
            取消
          </a>
          <form action="/oauth/authorize/approve" method="post" onSubmit={handleSubmit}>
            <input name="continue_to" type="hidden" value={props.bootstrap.continueTo} />
            <button className="auth-primary-button" disabled={submitting} type="submit">
              {submitting ? "继续中..." : "继续"}
            </button>
          </form>
        </div>
        <AuthFooterLinks />
      </section>
    </main>
  );
}

function DeviceAuthPage(props: { bootstrap: DeviceBootstrap }) {
  const [records, setRecords] = useState(props.bootstrap.records);
  const [message, setMessage] = useState(props.bootstrap.message);
  const [userCode, setUserCode] = useState("");
  const [submitting, setSubmitting] = useState(false);

  useEffect(() => {
    document.title = "Mock Device Auth";
    const interval = window.setInterval(async () => {
      const response = await fetch("/codex/device", {
        headers: { Accept: "application/json" },
        credentials: "same-origin",
      });
      const payload = (await response.json()) as DeviceBootstrap;
      setRecords(payload.records);
      setMessage((current) => current ?? payload.message);
    }, 2000);
    return () => window.clearInterval(interval);
  }, []);

  async function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setSubmitting(true);

    const response = await fetch("/codex/device", {
      method: "POST",
      headers: {
        Accept: "application/json",
        "Content-Type": "application/x-www-form-urlencoded",
      },
      body: new URLSearchParams({ user_code: userCode }).toString(),
      credentials: "same-origin",
    });
    const payload = (await response.json()) as DeviceBootstrap;
    setRecords(payload.records);
    setMessage(payload.message);
    setSubmitting(false);
    setUserCode("");
  }

  return (
    <Shell title="Device Authorization Console" eyebrow="Device Code Flow">
      <p className="lead">
        Open this page after <code>codex login --device-auth</code> prints a user code. The board refreshes automatically.
      </p>
      {message ? <p className="flash">{message}</p> : null}
      <form action="/codex/device" className="inline-form" method="post" onSubmit={handleSubmit}>
        <input
          name="user_code"
          placeholder="Enter the printed user code"
          value={userCode}
          onChange={(event) => setUserCode(event.target.value)}
        />
        <button className="primary-button" disabled={submitting} type="submit">
          {submitting ? "Approving..." : "Approve"}
        </button>
      </form>
      <div className="table-card">
        <div className="table-title">
          <strong>Active device codes</strong>
          <span>Polling every 2s</span>
        </div>
        <table>
          <thead>
            <tr>
              <th>User code</th>
              <th>Approved</th>
              <th>Polls</th>
            </tr>
          </thead>
          <tbody>
            {records.length === 0 ? (
              <tr>
                <td colSpan={3}>No active device codes yet.</td>
              </tr>
            ) : (
              records.map((record) => (
                <tr key={record.userCode}>
                  <td>
                    <code>{record.userCode}</code>
                  </td>
                  <td>{record.approved ? "yes" : "no"}</td>
                  <td>{record.polls}</td>
                </tr>
              ))
            )}
          </tbody>
        </table>
      </div>
    </Shell>
  );
}

function TaskPage(props: { bootstrap: TaskBootstrap }) {
  useEffect(() => {
    document.title = props.bootstrap.task?.title ?? "Unknown task";
  }, [props.bootstrap.task]);

  if (!props.bootstrap.task) {
    return (
      <Shell title="Unknown task" eyebrow="Task View">
        <p className="lead">
          The mock server does not know task <code>{props.bootstrap.missingTaskId}</code>.
        </p>
      </Shell>
    );
  }

  return (
    <Shell title={props.bootstrap.task.title} eyebrow="Task View">
      <p className="lead">
        Task ID <code>{props.bootstrap.task.taskId}</code>
      </p>
      <div className="content-grid">
        <section className="content-card">
          <h2>User prompt</h2>
          <pre>{props.bootstrap.task.userPrompt}</pre>
        </section>
        <section className="content-card">
          <h2>Assistant response</h2>
          <pre>{props.bootstrap.task.assistantResponse}</pre>
        </section>
      </div>
    </Shell>
  );
}

function CallbackPage() {
  useEffect(() => {
    document.title = "Mock Device Callback";
  }, []);

  return (
    <Shell title="Mock device callback reached" eyebrow="Device Callback">
      <p className="lead">The local device-auth flow redirected back successfully.</p>
    </Shell>
  );
}

function App() {
  const bootstrap = readBootstrap();

  if (bootstrap.page === "browserLogin") {
    return <BrowserLoginPage bootstrap={bootstrap} />;
  }
  if (bootstrap.page === "accountConfirm") {
    return <AccountConfirmPage bootstrap={bootstrap} />;
  }
  if (bootstrap.page === "deviceAuth") {
    return <DeviceAuthPage bootstrap={bootstrap} />;
  }
  if (bootstrap.page === "taskView") {
    return <TaskPage bootstrap={bootstrap} />;
  }
  return <CallbackPage />;
}

const root = document.getElementById("root");
if (!root) {
  throw new Error("missing root element");
}

createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
