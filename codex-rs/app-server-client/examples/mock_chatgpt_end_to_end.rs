use std::collections::HashMap;
use std::env;
use std::error::Error;
use std::fs;
use std::io::Read;
use std::io::Write;
use std::net::TcpStream;
use std::path::Path;
use std::path::PathBuf;
use std::process::Child;
use std::process::Command;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use codex_app_server_client::DEFAULT_IN_PROCESS_CHANNEL_CAPACITY;
use codex_app_server_client::InProcessAppServerClient;
use codex_app_server_client::InProcessClientStartArgs;
use codex_app_server_client::InProcessServerEvent;
use codex_app_server_protocol::Account;
use codex_app_server_protocol::AppsListParams;
use codex_app_server_protocol::AppsListResponse;
use codex_app_server_protocol::AskForApproval;
use codex_app_server_protocol::ClientRequest;
use codex_app_server_protocol::ConfigRequirementsReadResponse;
use codex_app_server_protocol::GetAccountParams;
use codex_app_server_protocol::GetAccountRateLimitsResponse;
use codex_app_server_protocol::GetAccountResponse;
use codex_app_server_protocol::LoginAccountParams;
use codex_app_server_protocol::LoginAccountResponse;
use codex_app_server_protocol::ModelListParams;
use codex_app_server_protocol::ModelListResponse;
use codex_app_server_protocol::RequestId;
use codex_app_server_protocol::SandboxMode;
use codex_app_server_protocol::ServerNotification;
use codex_app_server_protocol::ThreadStartParams;
use codex_app_server_protocol::ThreadStartResponse;
use codex_app_server_protocol::TurnStartParams;
use codex_app_server_protocol::TurnStartResponse;
use codex_app_server_protocol::TurnStatus;
use codex_app_server_protocol::UserInput;
use codex_arg0::Arg0DispatchPaths;
use codex_core::auth::AuthCredentialsStoreMode;
use codex_core::config::Config;
use codex_core::config::ConfigBuilder;
use codex_core::config_loader::CloudRequirementsLoader;
use codex_core::config_loader::LoaderOverrides;
use codex_core::features::Feature;
use codex_feedback::CodexFeedback;
use codex_protocol::protocol::SessionSource;
use toml::Value as TomlValue;
use url::Url;
use url::form_urlencoded;

type DynError = Box<dyn Error + Send + Sync>;

const LOGIN_USERNAME: &str = "debug@example.com";
const LOGIN_PASSWORD: &str = "debug-password";
const MOCK_SERVER_PORT: u16 = 8765;

fn main() -> Result<(), DynError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async_main())
}

async fn async_main() -> Result<(), DynError> {
    let temp_dir = TempDir::new("codex-app-server-client-mock-example")?;
    let codex_home = temp_dir.path().join("codex-home");
    let workspace = temp_dir.path().join("workspace");
    fs::create_dir_all(&codex_home)?;
    fs::create_dir_all(&workspace)?;

    let script_path = mock_server_script_path()?;
    let mock_port = MOCK_SERVER_PORT;
    // let _mock_server = MockServer::start(&script_path, mock_port)?;
    let refresh_url = format!("http://127.0.0.1:{mock_port}/oauth/token");
    let _refresh_url_guard = EnvVarGuard::set("CODEX_REFRESH_TOKEN_URL_OVERRIDE", &refresh_url);
    let openai_base_url = format!("http://127.0.0.1:{mock_port}/backend-api/codex");
    let _openai_base_url_guard = EnvVarGuard::set("OPENAI_BASE_URL", &openai_base_url);
    let cli_overrides = mock_cli_overrides(mock_port);

    let config =
        Arc::new(build_mock_config(mock_port, codex_home, workspace, &cli_overrides).await?);
    let mut client = InProcessAppServerClient::start(InProcessClientStartArgs {
        arg0_paths: Arg0DispatchPaths::default(),
        config,
        cli_overrides,
        loader_overrides: LoaderOverrides::default(),
        cloud_requirements: CloudRequirementsLoader::default(),
        feedback: CodexFeedback::new(),
        config_warnings: Vec::new(),
        session_source: SessionSource::Exec,
        enable_codex_api_key_env: false,
        client_name: "codex-app-server-client-example".to_string(),
        client_version: env!("CARGO_PKG_VERSION").to_string(),
        experimental_api: true,
        opt_out_notification_methods: Vec::new(),
        channel_capacity: DEFAULT_IN_PROCESS_CHANNEL_CAPACITY,
    })
    .await?;

    let login_response: LoginAccountResponse = client
        .request_typed(ClientRequest::LoginAccount {
            request_id: RequestId::Integer(1),
            params: LoginAccountParams::Chatgpt,
        })
        .await?;

    let (login_id, auth_url) = match login_response {
        LoginAccountResponse::Chatgpt { login_id, auth_url } => (login_id, auth_url),
        other => {
            return Err(format!("unexpected login response: {other:?}").into());
        }
    };
    println!("login auth url: {auth_url}");

    tokio::task::spawn_blocking(move || complete_browser_login(&auth_url))
        .await
        .map_err(|err| format!("browser login task failed: {err}"))??;
    wait_for_login_completion(&mut client, &login_id).await?;

    let account: GetAccountResponse = client
        .request_typed(ClientRequest::GetAccount {
            request_id: RequestId::Integer(2),
            params: GetAccountParams {
                refresh_token: false,
            },
        })
        .await?;
    print_account("account", &account);

    let refreshed_account: GetAccountResponse = client
        .request_typed(ClientRequest::GetAccount {
            request_id: RequestId::Integer(3),
            params: GetAccountParams {
                refresh_token: true,
            },
        })
        .await?;
    print_account("account after refresh", &refreshed_account);

    let rate_limits: GetAccountRateLimitsResponse = client
        .request_typed(ClientRequest::GetAccountRateLimits {
            request_id: RequestId::Integer(4),
            params: None,
        })
        .await?;
    println!(
        "rate limits: plan={:?}, primary={:?}",
        rate_limits.rate_limits.plan_type,
        rate_limits
            .rate_limits
            .primary
            .as_ref()
            .map(|window| window.used_percent)
    );

    let requirements: ConfigRequirementsReadResponse = client
        .request_typed(ClientRequest::ConfigRequirementsRead {
            request_id: RequestId::Integer(5),
            params: None,
        })
        .await?;
    println!(
        "requirements loaded: {}",
        requirements.requirements.is_some()
    );

    let models: ModelListResponse = client
        .request_typed(ClientRequest::ModelList {
            request_id: RequestId::Integer(6),
            params: ModelListParams::default(),
        })
        .await?;
    println!(
        "models: {}",
        models
            .data
            .iter()
            .map(|model| model.model.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );

    let apps: AppsListResponse = client
        .request_typed(ClientRequest::AppsList {
            request_id: RequestId::Integer(7),
            params: AppsListParams {
                force_refetch: true,
                ..AppsListParams::default()
            },
        })
        .await?;
    println!(
        "apps: {}",
        apps.data
            .iter()
            .map(|app| app.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );

    let thread: ThreadStartResponse = client
        .request_typed(ClientRequest::ThreadStart {
            request_id: RequestId::Integer(8),
            params: ThreadStartParams {
                approval_policy: Some(AskForApproval::Never),
                sandbox: Some(SandboxMode::DangerFullAccess),
                cwd: Some(temp_dir.path().join("workspace").display().to_string()),
                ephemeral: Some(true),
                ..ThreadStartParams::default()
            },
        })
        .await?;
    println!("thread started: {}", thread.thread.id);

    let turn: TurnStartResponse = client
        .request_typed(ClientRequest::TurnStart {
            request_id: RequestId::Integer(9),
            params: TurnStartParams {
                thread_id: thread.thread.id.clone(),
                input: vec![UserInput::Text {
                    text: "Use the mock ChatGPT account and say which endpoints were exercised."
                        .to_string(),
                    text_elements: Vec::new(),
                }],
                ..TurnStartParams::default()
            },
        })
        .await?;
    println!("turn started: {}", turn.turn.id);

    let assistant_text =
        wait_for_turn_completion(&mut client, &thread.thread.id, &turn.turn.id).await?;
    println!("assistant output: {assistant_text}");

    client.shutdown().await?;
    Ok(())
}

async fn build_mock_config(
    mock_port: u16,
    codex_home: PathBuf,
    workspace: PathBuf,
    cli_overrides: &[(String, TomlValue)],
) -> Result<Config, DynError> {
    let mut config = match ConfigBuilder::default()
        .cli_overrides(cli_overrides.to_vec())
        .build()
        .await
    {
        Ok(config) => config,
        Err(_) => Config::load_default_with_cli_overrides(cli_overrides.to_vec())?,
    };

    let backend_api_root = format!("http://127.0.0.1:{mock_port}/backend-api");
    let codex_base_url = format!("{backend_api_root}/codex");

    config.cwd = workspace;
    config.codex_home = codex_home.clone();
    config.sqlite_home = codex_home.join("sqlite");
    config.log_dir = codex_home.join("log");
    config.cli_auth_credentials_store_mode = AuthCredentialsStoreMode::File;
    config.chatgpt_base_url = backend_api_root;
    let _ = config.features.enable(Feature::Apps);
    config.model_provider_id = "openai".to_string();

    if let Some(provider) = config.model_providers.get_mut("openai") {
        provider.base_url = Some(codex_base_url.clone());
    }
    if let Some(provider) = config.model_providers.get("openai") {
        config.model_provider = provider.clone();
    } else {
        return Err("missing built-in openai model provider".into());
    }

    fs::create_dir_all(&config.sqlite_home)?;
    fs::create_dir_all(&config.log_dir)?;
    Ok(config)
}

fn mock_cli_overrides(mock_port: u16) -> Vec<(String, TomlValue)> {
    let backend_api_root = format!("http://127.0.0.1:{mock_port}/backend-api");
    vec![
        (
            "chatgpt_base_url".to_string(),
            TomlValue::String(backend_api_root),
        ),
        (
            "model_provider".to_string(),
            TomlValue::String("openai".to_string()),
        ),
        ("features.apps".to_string(), TomlValue::Boolean(true)),
        (
            "features.enable_request_compression".to_string(),
            TomlValue::Boolean(false),
        ),
    ]
}

async fn wait_for_login_completion(
    client: &mut InProcessAppServerClient,
    expected_login_id: &str,
) -> Result<(), DynError> {
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let event = client
                .next_event()
                .await
                .ok_or_else(|| "app-server event stream closed during login".to_string())?;
            match event {
                InProcessServerEvent::Lagged { skipped } => {
                    println!("lagged while waiting for login: skipped {skipped}");
                }
                InProcessServerEvent::ServerNotification(
                    ServerNotification::AccountLoginCompleted(notification),
                ) => {
                    if notification.login_id.as_deref() != Some(expected_login_id) {
                        continue;
                    }
                    if notification.success {
                        println!("login completed");
                        return Ok(());
                    }
                    return Err(notification
                        .error
                        .unwrap_or_else(|| "unknown login failure".to_string())
                        .into());
                }
                InProcessServerEvent::ServerNotification(ServerNotification::AccountUpdated(
                    notification,
                )) => {
                    println!(
                        "account updated: auth_mode={:?}, plan={:?}",
                        notification.auth_mode, notification.plan_type
                    );
                }
                InProcessServerEvent::ServerRequest(request) => {
                    return Err(
                        format!("unexpected server request during login: {request:?}").into(),
                    );
                }
                InProcessServerEvent::LegacyNotification(_)
                | InProcessServerEvent::ServerNotification(_) => {}
            }
        }
    })
    .await
    .map_err(|_| "timed out waiting for account/login/completed")?
}

async fn wait_for_turn_completion(
    client: &mut InProcessAppServerClient,
    thread_id: &str,
    turn_id: &str,
) -> Result<String, DynError> {
    tokio::time::timeout(Duration::from_secs(30), async {
        let mut assistant_text = String::new();
        loop {
            let event = client
                .next_event()
                .await
                .ok_or_else(|| "app-server event stream closed during turn".to_string())?;
            match event {
                InProcessServerEvent::Lagged { skipped } => {
                    println!("lagged while waiting for turn: skipped {skipped}");
                }
                InProcessServerEvent::ServerNotification(
                    ServerNotification::AgentMessageDelta(notification),
                ) if notification.thread_id == thread_id && notification.turn_id == turn_id => {
                    assistant_text.push_str(&notification.delta);
                }
                InProcessServerEvent::ServerNotification(ServerNotification::TurnCompleted(
                    notification,
                )) if notification.thread_id == thread_id && notification.turn.id == turn_id => {
                    match notification.turn.status {
                        TurnStatus::Completed => return Ok(assistant_text),
                        TurnStatus::Failed => {
                            let message = notification
                                .turn
                                .error
                                .as_ref()
                                .map(|error| error.message.clone())
                                .unwrap_or_else(|| "turn failed".to_string());
                            return Err(message.into());
                        }
                        status => {
                            return Err(format!("unexpected turn status: {status:?}").into());
                        }
                    }
                }
                InProcessServerEvent::ServerNotification(ServerNotification::Error(
                    notification,
                )) if notification.thread_id == thread_id && notification.turn_id == turn_id => {
                    return Err(notification.error.message.into());
                }
                InProcessServerEvent::ServerRequest(request) => {
                    return Err(
                        format!("unexpected server request during turn: {request:?}").into(),
                    );
                }
                InProcessServerEvent::LegacyNotification(_)
                | InProcessServerEvent::ServerNotification(_) => {}
            }
        }
    })
    .await
    .map_err(|_| "timed out waiting for turn/completed")?
}

fn print_account(label: &str, response: &GetAccountResponse) {
    match &response.account {
        Some(Account::Chatgpt { email, plan_type }) => {
            println!("{label}: chatgpt email={email}, plan={plan_type:?}");
        }
        Some(Account::ApiKey {}) => {
            println!("{label}: api key");
        }
        None => {
            println!(
                "{label}: no account, requires_openai_auth={}",
                response.requires_openai_auth
            );
        }
    }
}

fn complete_browser_login(auth_url: &str) -> Result<(), DynError> {
    let auth_url = Url::parse(auth_url)?;
    let continue_to = auth_url.path().to_string()
        + &auth_url
            .query()
            .map(|query| format!("?{query}"))
            .unwrap_or_default();
    let _login_page = http_request("GET", &auth_url, None, &[], None)?;

    let login_url = auth_url.join("/oauth/login")?;
    let form_body = form_urlencoded::Serializer::new(String::new())
        .append_pair("username", LOGIN_USERNAME)
        .append_pair("password", LOGIN_PASSWORD)
        .append_pair("continue_to", &continue_to)
        .finish();
    let login_response = http_request(
        "POST",
        &login_url,
        None,
        form_body.as_bytes(),
        Some("application/x-www-form-urlencoded"),
    )?;
    if login_response.status_code != 302 {
        return Err(format!(
            "mock login expected 302, got {} with body {}",
            login_response.status_code,
            String::from_utf8_lossy(&login_response.body)
        )
        .into());
    }

    let cookie = login_response
        .header("set-cookie")
        .and_then(|value| value.split(';').next())
        .map(str::to_string)
        .ok_or_else(|| "mock login did not return a session cookie".to_string())?;
    let location = login_response
        .header("location")
        .ok_or_else(|| "mock login did not return a redirect".to_string())?;
    follow_redirects(login_url.join(location)?, Some(cookie))?;
    Ok(())
}

fn follow_redirects(mut url: Url, cookie: Option<String>) -> Result<(), DynError> {
    for _ in 0..10 {
        let response = http_request("GET", &url, cookie.as_deref(), &[], None)?;
        if !(300..400).contains(&response.status_code) {
            return Ok(());
        }
        let location = response
            .header("location")
            .ok_or_else(|| format!("redirect from {url} missing Location header"))?;
        url = url.join(location)?;
    }
    Err("too many redirects while completing browser login".into())
}

fn http_request(
    method: &str,
    url: &Url,
    cookie: Option<&str>,
    body: &[u8],
    content_type: Option<&str>,
) -> Result<HttpResponse, DynError> {
    if url.scheme() != "http" {
        return Err(format!("only http URLs are supported in this example: {url}").into());
    }

    let host = url
        .host_str()
        .ok_or_else(|| format!("missing host in URL: {url}"))?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| format!("missing port in URL: {url}"))?;
    let mut stream = TcpStream::connect((host, port))?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;

    let mut request = format!(
        "{method} {} HTTP/1.1\r\nHost: {host}:{port}\r\nConnection: close\r\n",
        request_path(url)
    );
    if let Some(cookie) = cookie {
        request.push_str(&format!("Cookie: {cookie}\r\n"));
    }
    if let Some(content_type) = content_type {
        request.push_str(&format!("Content-Type: {content_type}\r\n"));
    }
    if !body.is_empty() {
        request.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    request.push_str("\r\n");

    stream.write_all(request.as_bytes())?;
    if !body.is_empty() {
        stream.write_all(body)?;
    }
    stream.flush()?;

    let mut response_bytes = Vec::new();
    stream.read_to_end(&mut response_bytes)?;
    HttpResponse::parse(&response_bytes)
}

fn request_path(url: &Url) -> String {
    match url.query() {
        Some(query) => format!("{}?{query}", url.path()),
        None => url.path().to_string(),
    }
}

fn mock_server_script_path() -> Result<PathBuf, DynError> {
    let default_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../scripts/mock_chatgpt_account_server.py")
        .canonicalize()?;
    Ok(match env::var("CODEX_MOCK_CHATGPT_SERVER_PATH") {
        Ok(value) => PathBuf::from(value),
        Err(_) => default_path,
    })
}

struct MockServer {
    child: Child,
}

impl MockServer {
    fn start(script_path: &Path, port: u16) -> Result<Self, DynError> {
        let mut last_not_found = None;
        for candidate in python_candidates() {
            match spawn_mock_server(script_path, port, &candidate) {
                Ok(mut child) => {
                    wait_for_mock_server_ready(port, &mut child)?;
                    return Ok(Self { child });
                }
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                    last_not_found = Some(err);
                }
                Err(err) => return Err(err.into()),
            }
        }

        match last_not_found {
            Some(err) => Err(format!("failed to find a Python interpreter: {err}").into()),
            None => Err("failed to spawn mock server".into()),
        }
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[derive(Clone)]
struct PythonCandidate {
    program: String,
    args: Vec<String>,
}

fn python_candidates() -> Vec<PythonCandidate> {
    let mut candidates = Vec::new();
    if let Ok(program) = env::var("PYTHON") {
        candidates.push(PythonCandidate {
            program,
            args: Vec::new(),
        });
    }
    candidates.push(PythonCandidate {
        program: "python".to_string(),
        args: Vec::new(),
    });
    candidates.push(PythonCandidate {
        program: "python3".to_string(),
        args: Vec::new(),
    });
    if cfg!(windows) {
        candidates.push(PythonCandidate {
            program: "py".to_string(),
            args: vec!["-3".to_string()],
        });
    }
    candidates
}

fn spawn_mock_server(
    script_path: &Path,
    port: u16,
    candidate: &PythonCandidate,
) -> Result<Child, std::io::Error> {
    let mut command = Command::new(&candidate.program);
    command
        .args(&candidate.args)
        .arg(script_path)
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(port.to_string())
        .arg("--login-username")
        .arg(LOGIN_USERNAME)
        .arg("--login-password")
        .arg(LOGIN_PASSWORD)
        .arg("--device-code-auto-approve")
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    command.spawn()
}

fn wait_for_mock_server_ready(port: u16, child: &mut Child) -> Result<(), DynError> {
    let health_url = Url::parse(&format!("http://127.0.0.1:{port}/healthz"))?;
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(10) {
        if let Some(status) = child.try_wait()? {
            return Err(format!("mock server exited before readiness check: {status}").into());
        }
        if let Ok(response) = http_request("GET", &health_url, None, &[], None)
            && response.status_code == 200
        {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Err("timed out waiting for mock server /healthz".into())
}

struct EnvVarGuard {
    key: String,
    original: Option<String>,
}

impl EnvVarGuard {
    fn set(key: &str, value: &str) -> Self {
        let original = env::var(key).ok();
        // SAFETY: This example mutates the process environment before the
        // app-server runtime starts reading refresh-token settings, and it
        // restores the prior value only after shutdown.
        unsafe {
            env::set_var(key, value);
        }
        Self {
            key: key.to_string(),
            original,
        }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        // SAFETY: The original value is restored during example teardown after
        // the app-server client has been shut down.
        unsafe {
            if let Some(value) = &self.original {
                env::set_var(&self.key, value);
            } else {
                env::remove_var(&self.key);
            }
        }
    }
}

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(prefix: &str) -> Result<Self, DynError> {
        let unique = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let path = env::temp_dir().join(format!("{prefix}-{unique}"));
        fs::create_dir_all(&path)?;
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[derive(Debug)]
struct HttpResponse {
    status_code: u16,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

impl HttpResponse {
    fn parse(bytes: &[u8]) -> Result<Self, DynError> {
        let header_end = bytes
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .ok_or_else(|| "invalid HTTP response: missing header terminator".to_string())?;
        let header_bytes = &bytes[..header_end];
        let body = bytes[header_end + 4..].to_vec();
        let header_text = String::from_utf8(header_bytes.to_vec())?;
        let mut lines = header_text.split("\r\n");
        let status_line = lines
            .next()
            .ok_or_else(|| "invalid HTTP response: missing status line".to_string())?;
        let mut status_parts = status_line.split_whitespace();
        let _http_version = status_parts
            .next()
            .ok_or_else(|| "invalid HTTP response: missing version".to_string())?;
        let status_code = status_parts
            .next()
            .ok_or_else(|| "invalid HTTP response: missing status code".to_string())?
            .parse::<u16>()?;

        let mut headers = HashMap::new();
        for line in lines {
            if let Some((name, value)) = line.split_once(':') {
                headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
            }
        }

        Ok(Self {
            status_code,
            headers,
            body,
        })
    }

    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }
}
