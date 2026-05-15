use axum::Json;
use axum::Router;
use axum::extract::Path;
use axum::extract::State;
use axum::extract::ws::Message;
use axum::extract::ws::WebSocket;
use axum::extract::ws::WebSocketUpgrade;
use axum::http::HeaderMap;
use axum::response::Html;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::routing::post;
use base64::Engine;
use futures::SinkExt;
use futures::StreamExt;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::io;
use std::io::ErrorKind;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio::sync::mpsc;
use uuid::Uuid;

const CONTROLLER_HTML_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/examples/remote_controller.html"
);
const REMOTE_CONTROL_ACCOUNT_ID_HEADER: &str = "chatgpt-account-id";
const REMOTE_CONTROL_INSTALLATION_ID_HEADER: &str = "x-codex-installation-id";
const REMOTE_CONTROL_SERVER_ID_HEADER: &str = "x-codex-server-id";
const REMOTE_CONTROL_SERVER_NAME_HEADER: &str = "x-codex-name";

const REMOTE_CONTROL_SEGMENT_TARGET_BYTES: usize = 100 * 1024;
const REMOTE_CONTROL_SEGMENT_MAX_BYTES: usize = 150 * 1024;
const REMOTE_CONTROL_REASSEMBLED_MAX_BYTES: usize = 100 * 1024 * 1024;
const REMOTE_CONTROL_SEGMENT_COUNT_MAX: usize = 1024;

#[derive(Clone, Default)]
struct AppState {
    inner: Arc<Mutex<SharedState>>,
}

#[derive(Default)]
struct SharedState {
    environments: HashMap<String, Arc<EnvironmentState>>,
    server_id_to_environment: HashMap<String, String>,
}

struct EnvironmentState {
    environment_id: String,
    server_id: String,
    installation_id: String,
    account_id: String,
    server_name: Mutex<String>,
    codex_sender: Mutex<Option<mpsc::UnboundedSender<Message>>>,
    controllers: Mutex<HashMap<String, Arc<ControllerConnection>>>,
}

impl EnvironmentState {
    fn new(
        environment_id: String,
        server_id: String,
        installation_id: String,
        account_id: String,
        server_name: String,
    ) -> Self {
        Self {
            environment_id,
            server_id,
            installation_id,
            account_id,
            server_name: Mutex::new(server_name),
            codex_sender: Mutex::new(None),
            controllers: Mutex::new(HashMap::new()),
        }
    }
}

struct ControllerConnection {
    controller_id: String,
    client_id: String,
    stream_id: String,
    next_seq_id: AtomicU64,
    sender: mpsc::UnboundedSender<Message>,
    reassembler: Mutex<ServerSegmentReassembler>,
}

impl ControllerConnection {
    fn new(sender: mpsc::UnboundedSender<Message>) -> Self {
        Self {
            controller_id: format!("controller-{}", Uuid::now_v7()),
            client_id: format!("browser-{}", Uuid::now_v7()),
            stream_id: Uuid::now_v7().to_string(),
            next_seq_id: AtomicU64::new(1),
            sender,
            reassembler: Mutex::new(ServerSegmentReassembler::default()),
        }
    }

    fn next_seq_id(&self) -> u64 {
        self.next_seq_id.fetch_add(1, Ordering::Relaxed)
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EnrollRequest {
    name: String,
    os: String,
    arch: String,
    app_server_version: String,
    installation_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct EnrollResponse {
    server_id: String,
    environment_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct EnvironmentSummary {
    environment_id: String,
    server_id: String,
    installation_id: String,
    account_id: String,
    server_name: String,
    codex_connected: bool,
    controller_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
struct ClientEnvelope {
    #[serde(flatten)]
    event: ClientEvent,
    client_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    seq_id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientEvent {
    ClientMessage {
        message: Value,
    },
    ClientMessageChunk {
        segment_id: usize,
        segment_count: usize,
        message_size_bytes: usize,
        message_chunk_base64: String,
    },
    Ack {
        #[serde(skip_serializing_if = "Option::is_none")]
        segment_id: Option<usize>,
    },
    Ping,
    ClientClosed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
struct ServerEnvelope {
    #[serde(flatten)]
    event: ServerEvent,
    client_id: String,
    stream_id: String,
    seq_id: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerEvent {
    ServerMessage {
        message: Value,
    },
    ServerMessageChunk {
        segment_id: usize,
        segment_count: usize,
        message_size_bytes: usize,
        message_chunk_base64: String,
    },
    Ack,
    Pong {
        status: String,
    },
}

impl ServerEvent {
    fn segment_id(&self) -> Option<usize> {
        match self {
            Self::ServerMessageChunk { segment_id, .. } => Some(*segment_id),
            Self::ServerMessage { .. } | Self::Ack | Self::Pong { .. } => None,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum ControllerCommand {
    Initialize {
        request_id: Option<i64>,
        client_name: Option<String>,
        client_version: Option<String>,
        experimental_api: Option<bool>,
        request_attestation: Option<bool>,
    },
    Initialized,
    Rpc {
        message: Value,
    },
    Ping,
    Close,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum ControllerEvent {
    Hello {
        environment_id: String,
        server_id: String,
        client_id: String,
        stream_id: String,
        codex_connected: bool,
    },
    Status {
        codex_connected: bool,
        controller_count: usize,
    },
    CodexEnvelope {
        envelope: ServerEnvelope,
    },
    Error {
        message: String,
    },
}

#[derive(Default)]
struct ServerSegmentReassembler {
    assembly: Option<ServerSegmentAssembly>,
}

struct ServerSegmentAssembly {
    seq_id: u64,
    stream_id: String,
    segment_count: usize,
    message_size_bytes: usize,
    raw: Vec<u8>,
    next_segment_id: usize,
}

enum ServerSegmentObservation {
    Forward(ServerEnvelope),
    Pending,
    Dropped,
}

#[derive(Default)]
struct CountingWriter {
    len: usize,
}

impl Write for CountingWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.len += buf.len();
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let state = AppState::default();
    let app = Router::new()
        .route("/", get(index))
        .route("/environments", get(list_environments))
        .route(
            "/backend-api/wham/remote/control/server/enroll",
            post(enroll_remote_server),
        )
        .route(
            "/backend-api/wham/remote/control/server",
            get(remote_control_server_ws),
        )
        .route("/controller/ws/{environment_id}", get(controller_ws))
        .with_state(state);

    let listener = TcpListener::bind("127.0.0.1:8787").await?;
    println!("remote control demo backend listening on http://127.0.0.1:8787");
    println!("controller page: http://127.0.0.1:8787/");
    println!("set chatgpt_base_url = \"http://127.0.0.1:8787/backend-api/\"");
    println!("Codex remote-control still requires ChatGPT login in the current implementation.");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn index() -> impl IntoResponse {
    match tokio::fs::read_to_string(PathBuf::from(CONTROLLER_HTML_PATH)).await {
        Ok(contents) => Html(contents).into_response(),
        Err(err) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to read controller html: {err}"),
        )
            .into_response(),
    }
}

async fn list_environments(State(state): State<AppState>) -> Json<Vec<EnvironmentSummary>> {
    let environments = {
        let guard = state.inner.lock().await;
        guard.environments.values().cloned().collect::<Vec<_>>()
    };

    let mut summaries = Vec::with_capacity(environments.len());
    for environment in environments {
        let server_name = environment.server_name.lock().await.clone();
        let codex_connected = environment.codex_sender.lock().await.is_some();
        let controller_count = environment.controllers.lock().await.len();
        summaries.push(EnvironmentSummary {
            environment_id: environment.environment_id.clone(),
            server_id: environment.server_id.clone(),
            installation_id: environment.installation_id.clone(),
            account_id: environment.account_id.clone(),
            server_name,
            codex_connected,
            controller_count,
        });
    }
    Json(summaries)
}

async fn enroll_remote_server(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<EnrollRequest>,
) -> impl IntoResponse {
    let EnrollRequest {
        name: server_name,
        os: _os,
        arch: _arch,
        app_server_version: _app_server_version,
        installation_id: request_installation_id,
    } = request;
    let account_id = header_string(&headers, REMOTE_CONTROL_ACCOUNT_ID_HEADER)
        .unwrap_or_else(|| "account-demo".to_string());
    let installation_id = header_string(&headers, REMOTE_CONTROL_INSTALLATION_ID_HEADER)
        .unwrap_or(request_installation_id);

    let mut guard = state.inner.lock().await;
    if let Some(existing) = guard.environments.values().find(|environment| {
        environment.installation_id == installation_id && environment.account_id == account_id
    }) {
        *existing.server_name.lock().await = server_name;
        return Json(EnrollResponse {
            server_id: existing.server_id.clone(),
            environment_id: existing.environment_id.clone(),
        });
    }

    let environment_id = format!("env_{}", Uuid::now_v7().simple());
    let server_id = format!("srv_{}", Uuid::now_v7().simple());
    let environment = Arc::new(EnvironmentState::new(
        environment_id.clone(),
        server_id.clone(),
        installation_id,
        account_id,
        server_name,
    ));
    guard
        .server_id_to_environment
        .insert(server_id.clone(), environment_id.clone());
    guard
        .environments
        .insert(environment_id.clone(), environment);
    Json(EnrollResponse {
        server_id,
        environment_id,
    })
}

async fn remote_control_server_ws(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let Some(server_id) = header_string(&headers, REMOTE_CONTROL_SERVER_ID_HEADER) else {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            "missing x-codex-server-id header".to_string(),
        )
            .into_response();
    };
    let environment = {
        let guard = state.inner.lock().await;
        let Some(environment_id) = guard.server_id_to_environment.get(&server_id) else {
            return (
                axum::http::StatusCode::NOT_FOUND,
                format!("unknown server id `{server_id}`"),
            )
                .into_response();
        };
        let Some(environment) = guard.environments.get(environment_id) else {
            return (
                axum::http::StatusCode::NOT_FOUND,
                format!("missing environment for server id `{server_id}`"),
            )
                .into_response();
        };
        environment.clone()
    };
    let server_name = decode_server_name_header(&headers);

    ws.on_upgrade(move |socket| async move {
        handle_codex_socket(socket, environment, server_name).await;
    })
    .into_response()
}

async fn controller_ws(
    ws: WebSocketUpgrade,
    Path(environment_id): Path<String>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let environment = {
        let guard = state.inner.lock().await;
        let Some(environment) = guard.environments.get(&environment_id) else {
            return (
                axum::http::StatusCode::NOT_FOUND,
                format!("unknown environment `{environment_id}`"),
            )
                .into_response();
        };
        environment.clone()
    };

    ws.on_upgrade(move |socket| async move {
        handle_controller_socket(socket, environment).await;
    })
    .into_response()
}

async fn handle_codex_socket(
    socket: WebSocket,
    environment: Arc<EnvironmentState>,
    server_name: Option<String>,
) {
    if let Some(server_name) = server_name {
        *environment.server_name.lock().await = server_name;
    }

    let (mut sender, mut receiver) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<Message>();
    {
        let mut codex_sender = environment.codex_sender.lock().await;
        *codex_sender = Some(tx);
    }
    broadcast_status(&environment).await;

    let writer_task = tokio::spawn(async move {
        while let Some(message) = rx.recv().await {
            if sender.send(message).await.is_err() {
                break;
            }
        }
    });

    while let Some(message_result) = receiver.next().await {
        let Ok(message) = message_result else {
            break;
        };
        let Message::Text(text) = message else {
            continue;
        };
        let Ok(envelope) = serde_json::from_str::<ServerEnvelope>(text.as_str()) else {
            broadcast_error(
                &environment,
                format!(
                    "failed to decode codex websocket payload: {}",
                    text.as_str()
                ),
            )
            .await;
            continue;
        };

        ack_codex_envelope(&environment, &envelope).await;

        let Some(controller) =
            find_controller_for_route(&environment, &envelope.client_id, &envelope.stream_id).await
        else {
            broadcast_error(
                &environment,
                format!(
                    "received codex message for unknown route client_id={} stream_id={}",
                    envelope.client_id, envelope.stream_id
                ),
            )
            .await;
            continue;
        };

        let observation = controller.reassembler.lock().await.observe(envelope);
        match observation {
            ServerSegmentObservation::Forward(envelope) => {
                let _ = send_event_to_controller(
                    &controller,
                    &ControllerEvent::CodexEnvelope { envelope },
                );
            }
            ServerSegmentObservation::Pending | ServerSegmentObservation::Dropped => {}
        }
    }

    {
        let mut codex_sender = environment.codex_sender.lock().await;
        *codex_sender = None;
    }
    writer_task.abort();
    broadcast_status(&environment).await;
}

async fn handle_controller_socket(socket: WebSocket, environment: Arc<EnvironmentState>) {
    let (mut sender, mut receiver) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<Message>();
    let controller = Arc::new(ControllerConnection::new(tx));

    {
        let mut controllers = environment.controllers.lock().await;
        controllers.insert(controller.controller_id.clone(), controller.clone());
    }

    let hello = ControllerEvent::Hello {
        environment_id: environment.environment_id.clone(),
        server_id: environment.server_id.clone(),
        client_id: controller.client_id.clone(),
        stream_id: controller.stream_id.clone(),
        codex_connected: environment.codex_sender.lock().await.is_some(),
    };
    let _ = send_event_to_controller(&controller, &hello);
    broadcast_status(&environment).await;

    let writer_task = tokio::spawn(async move {
        while let Some(message) = rx.recv().await {
            if sender.send(message).await.is_err() {
                break;
            }
        }
    });

    while let Some(message_result) = receiver.next().await {
        let Ok(message) = message_result else {
            break;
        };
        let Message::Text(text) = message else {
            continue;
        };
        match serde_json::from_str::<ControllerCommand>(text.as_str()) {
            Ok(command) => {
                if let Err(err) =
                    handle_controller_command(&environment, controller.clone(), command).await
                {
                    let _ = send_event_to_controller(
                        &controller,
                        &ControllerEvent::Error { message: err },
                    );
                }
            }
            Err(err) => {
                let _ = send_event_to_controller(
                    &controller,
                    &ControllerEvent::Error {
                        message: format!("invalid controller command: {err}"),
                    },
                );
            }
        }
    }

    {
        let mut controllers = environment.controllers.lock().await;
        controllers.remove(&controller.controller_id);
    }
    writer_task.abort();
    broadcast_status(&environment).await;
}

async fn handle_controller_command(
    environment: &Arc<EnvironmentState>,
    controller: Arc<ControllerConnection>,
    command: ControllerCommand,
) -> Result<(), String> {
    match command {
        ControllerCommand::Initialize {
            request_id,
            client_name,
            client_version,
            experimental_api,
            request_attestation,
        } => {
            let request = serde_json::json!({
                "jsonrpc": "2.0",
                "id": request_id.unwrap_or(1),
                "method": "initialize",
                "params": {
                    "clientInfo": {
                        "name": client_name.unwrap_or_else(|| "remote-controller-demo".to_string()),
                        "title": "Remote Controller Demo",
                        "version": client_version.unwrap_or_else(|| "0.1.0".to_string())
                    },
                    "capabilities": {
                        "experimentalApi": experimental_api.unwrap_or(true),
                        "requestAttestation": request_attestation.unwrap_or(false)
                    }
                }
            });
            send_jsonrpc_to_codex(environment, &controller, request).await
        }
        ControllerCommand::Initialized => {
            let notification = serde_json::json!({
                "jsonrpc": "2.0",
                "method": "initialized"
            });
            send_jsonrpc_to_codex(environment, &controller, notification).await
        }
        ControllerCommand::Rpc { message } => {
            send_jsonrpc_to_codex(environment, &controller, message).await
        }
        ControllerCommand::Ping => {
            send_single_envelope_to_codex(
                environment,
                ClientEnvelope {
                    event: ClientEvent::Ping,
                    client_id: controller.client_id.clone(),
                    stream_id: None,
                    seq_id: None,
                    cursor: None,
                },
            )
            .await
        }
        ControllerCommand::Close => {
            send_single_envelope_to_codex(
                environment,
                ClientEnvelope {
                    event: ClientEvent::ClientClosed,
                    client_id: controller.client_id.clone(),
                    stream_id: Some(controller.stream_id.clone()),
                    seq_id: None,
                    cursor: None,
                },
            )
            .await
        }
    }
}

async fn send_jsonrpc_to_codex(
    environment: &Arc<EnvironmentState>,
    controller: &Arc<ControllerConnection>,
    message: Value,
) -> Result<(), String> {
    let seq_id = controller.next_seq_id();
    let envelopes = split_client_message_for_transport(
        &controller.client_id,
        &controller.stream_id,
        seq_id,
        &message,
    )?;

    for envelope in envelopes {
        send_single_envelope_to_codex(environment, envelope).await?;
    }
    Ok(())
}

async fn send_single_envelope_to_codex(
    environment: &Arc<EnvironmentState>,
    envelope: ClientEnvelope,
) -> Result<(), String> {
    let payload = serde_json::to_string(&envelope)
        .map_err(|err| format!("failed to serialize controller envelope: {err}"))?;
    let codex_sender = environment
        .codex_sender
        .lock()
        .await
        .clone()
        .ok_or_else(|| "codex is not connected for this environment".to_string())?;
    codex_sender
        .send(Message::Text(payload.into()))
        .map_err(|_| "failed to send message to codex websocket".to_string())
}

async fn ack_codex_envelope(environment: &Arc<EnvironmentState>, envelope: &ServerEnvelope) {
    let ack = ClientEnvelope {
        event: ClientEvent::Ack {
            segment_id: envelope.event.segment_id(),
        },
        client_id: envelope.client_id.clone(),
        stream_id: Some(envelope.stream_id.clone()),
        seq_id: Some(envelope.seq_id),
        cursor: None,
    };
    let _ = send_single_envelope_to_codex(environment, ack).await;
}

async fn find_controller_for_route(
    environment: &Arc<EnvironmentState>,
    client_id: &str,
    stream_id: &str,
) -> Option<Arc<ControllerConnection>> {
    let controllers = environment.controllers.lock().await;
    controllers
        .values()
        .find(|controller| controller.client_id == client_id && controller.stream_id == stream_id)
        .cloned()
}

async fn broadcast_status(environment: &Arc<EnvironmentState>) {
    let codex_connected = environment.codex_sender.lock().await.is_some();
    let controller_count = environment.controllers.lock().await.len();
    broadcast_event(
        environment,
        &ControllerEvent::Status {
            codex_connected,
            controller_count,
        },
    )
    .await;
}

async fn broadcast_error(environment: &Arc<EnvironmentState>, message: String) {
    broadcast_event(environment, &ControllerEvent::Error { message }).await;
}

async fn broadcast_event(environment: &Arc<EnvironmentState>, event: &ControllerEvent) {
    let controllers = {
        let controllers = environment.controllers.lock().await;
        controllers.values().cloned().collect::<Vec<_>>()
    };
    for controller in controllers {
        let _ = send_event_to_controller(&controller, event);
    }
}

fn send_event_to_controller(
    controller: &Arc<ControllerConnection>,
    event: &ControllerEvent,
) -> Result<(), String> {
    let payload = serde_json::to_string(event)
        .map_err(|err| format!("failed to serialize controller event: {err}"))?;
    controller
        .sender
        .send(Message::Text(payload.into()))
        .map_err(|_| "failed to send websocket message to controller".to_string())
}

impl ServerSegmentReassembler {
    fn observe(&mut self, envelope: ServerEnvelope) -> ServerSegmentObservation {
        let ServerEvent::ServerMessageChunk {
            segment_id,
            segment_count,
            message_size_bytes,
            message_chunk_base64,
        } = &envelope.event
        else {
            return ServerSegmentObservation::Forward(envelope);
        };
        let segment_id = *segment_id;
        let segment_count = *segment_count;
        let message_size_bytes = *message_size_bytes;

        if segment_count == 0
            || segment_count > REMOTE_CONTROL_SEGMENT_COUNT_MAX
            || segment_id >= segment_count
            || message_size_bytes == 0
            || message_size_bytes > REMOTE_CONTROL_REASSEMBLED_MAX_BYTES
            || message_chunk_base64.is_empty()
        {
            self.assembly = None;
            return ServerSegmentObservation::Dropped;
        }

        match &self.assembly {
            Some(assembly)
                if assembly.stream_id == envelope.stream_id
                    && assembly.seq_id == envelope.seq_id => {}
            Some(assembly) if envelope.seq_id < assembly.seq_id => {
                return ServerSegmentObservation::Dropped;
            }
            _ => {
                if segment_id != 0 {
                    self.assembly = None;
                    return ServerSegmentObservation::Dropped;
                }
                self.assembly = Some(ServerSegmentAssembly {
                    seq_id: envelope.seq_id,
                    stream_id: envelope.stream_id.clone(),
                    segment_count,
                    message_size_bytes,
                    raw: Vec::with_capacity(message_size_bytes),
                    next_segment_id: 0,
                });
            }
        }

        let Some(assembly) = self.assembly.as_mut() else {
            return ServerSegmentObservation::Dropped;
        };
        if assembly.segment_count != segment_count
            || assembly.message_size_bytes != message_size_bytes
            || assembly.stream_id != envelope.stream_id
        {
            self.assembly = None;
            return ServerSegmentObservation::Dropped;
        }
        if segment_id < assembly.next_segment_id {
            return ServerSegmentObservation::Dropped;
        }
        if segment_id != assembly.next_segment_id {
            self.assembly = None;
            return ServerSegmentObservation::Dropped;
        }

        let chunk_start = assembly.raw.len();
        let decoded_len = base64::decoded_len_estimate(message_chunk_base64.len());
        let chunk_end = usize::min(message_size_bytes, chunk_start.saturating_add(decoded_len));
        assembly.raw.resize(chunk_end, 0);
        let decode_result = base64::engine::general_purpose::STANDARD.decode_slice(
            message_chunk_base64.as_bytes(),
            &mut assembly.raw[chunk_start..],
        );
        let Ok(decoded_chunk_len) = decode_result else {
            self.assembly = None;
            return ServerSegmentObservation::Dropped;
        };
        assembly.raw.truncate(chunk_start + decoded_chunk_len);
        assembly.next_segment_id += 1;

        if assembly.next_segment_id < segment_count {
            return ServerSegmentObservation::Pending;
        }
        if assembly.raw.len() != message_size_bytes {
            self.assembly = None;
            return ServerSegmentObservation::Dropped;
        }
        let Ok(message) = serde_json::from_slice::<Value>(&assembly.raw) else {
            self.assembly = None;
            return ServerSegmentObservation::Dropped;
        };
        self.assembly = None;
        ServerSegmentObservation::Forward(ServerEnvelope {
            event: ServerEvent::ServerMessage { message },
            client_id: envelope.client_id,
            stream_id: envelope.stream_id,
            seq_id: envelope.seq_id,
        })
    }
}

fn split_client_message_for_transport(
    client_id: &str,
    stream_id: &str,
    seq_id: u64,
    message: &Value,
) -> Result<Vec<ClientEnvelope>, String> {
    let direct_envelope = ClientEnvelope {
        event: ClientEvent::ClientMessage {
            message: message.clone(),
        },
        client_id: client_id.to_string(),
        stream_id: Some(stream_id.to_string()),
        seq_id: Some(seq_id),
        cursor: None,
    };
    let direct_size_bytes = serialized_len(&direct_envelope)
        .map_err(|err| format!("failed to measure client envelope size: {err}"))?;
    if direct_size_bytes <= REMOTE_CONTROL_SEGMENT_MAX_BYTES {
        return Ok(vec![direct_envelope]);
    }

    let raw = serde_json::to_vec(message)
        .map_err(|err| format!("failed to serialize JSON-RPC message for chunking: {err}"))?;
    let message_size_bytes = raw.len();
    if message_size_bytes > REMOTE_CONTROL_REASSEMBLED_MAX_BYTES {
        return Err("client message exceeds remote-control reassembled size limit".to_string());
    }

    let minimal_segment_count =
        usize::min(message_size_bytes.max(1), REMOTE_CONTROL_SEGMENT_COUNT_MAX);
    let minimal_chunk = &raw[..usize::min(raw.len(), 1)];
    if serialized_client_chunk_len(
        client_id,
        stream_id,
        seq_id,
        0,
        minimal_segment_count,
        message_size_bytes,
        minimal_chunk,
    )
    .map_err(|err| format!("failed to size client chunk envelope: {err}"))?
        > REMOTE_CONTROL_SEGMENT_MAX_BYTES
    {
        return Err(
            "client message cannot fit within remote-control segment size limit".to_string(),
        );
    }

    let mut segment_count = usize::max(
        2,
        message_size_bytes.div_ceil(REMOTE_CONTROL_SEGMENT_TARGET_BYTES),
    );
    loop {
        let chunk_size = usize::max(1, message_size_bytes.div_ceil(segment_count));
        segment_count = message_size_bytes.div_ceil(chunk_size);
        let fits = raw
            .chunks(chunk_size)
            .enumerate()
            .all(|(segment_id, chunk)| {
                serialized_client_chunk_len(
                    client_id,
                    stream_id,
                    seq_id,
                    segment_id,
                    segment_count,
                    message_size_bytes,
                    chunk,
                )
                .is_ok_and(|size| size <= REMOTE_CONTROL_SEGMENT_MAX_BYTES)
            });
        if fits {
            return raw
                .chunks(chunk_size)
                .enumerate()
                .map(|(segment_id, chunk)| {
                    build_client_chunk_envelope(
                        client_id,
                        stream_id,
                        seq_id,
                        segment_id,
                        segment_count,
                        message_size_bytes,
                        chunk,
                    )
                })
                .collect::<io::Result<Vec<_>>>()
                .map_err(|err| format!("failed to build client chunk envelopes: {err}"));
        }
        if chunk_size == 1 {
            return Err(
                "client message cannot fit within remote-control segment size limit".to_string(),
            );
        }
        let next_segment_count = segment_count + 1;
        let next_chunk_size = usize::max(1, message_size_bytes.div_ceil(next_segment_count));
        segment_count = if next_chunk_size == chunk_size {
            message_size_bytes
        } else {
            next_segment_count
        };
    }
}

fn serialized_client_chunk_len(
    client_id: &str,
    stream_id: &str,
    seq_id: u64,
    segment_id: usize,
    segment_count: usize,
    message_size_bytes: usize,
    chunk: &[u8],
) -> io::Result<usize> {
    serialized_len(&build_client_chunk_envelope(
        client_id,
        stream_id,
        seq_id,
        segment_id,
        segment_count,
        message_size_bytes,
        chunk,
    )?)
}

fn build_client_chunk_envelope(
    client_id: &str,
    stream_id: &str,
    seq_id: u64,
    segment_id: usize,
    segment_count: usize,
    message_size_bytes: usize,
    chunk: &[u8],
) -> io::Result<ClientEnvelope> {
    if segment_count > REMOTE_CONTROL_SEGMENT_COUNT_MAX {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "remote-control client segment count exceeds maximum",
        ));
    }
    Ok(ClientEnvelope {
        event: ClientEvent::ClientMessageChunk {
            segment_id,
            segment_count,
            message_size_bytes,
            message_chunk_base64: base64::engine::general_purpose::STANDARD.encode(chunk),
        },
        client_id: client_id.to_string(),
        stream_id: Some(stream_id.to_string()),
        seq_id: Some(seq_id),
        cursor: None,
    })
}

fn serialized_len(value: &impl serde::Serialize) -> io::Result<usize> {
    let mut writer = CountingWriter::default();
    serde_json::to_writer(&mut writer, value).map_err(io::Error::other)?;
    Ok(writer.len)
}

fn header_string(headers: &HeaderMap, name: &'static str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn decode_server_name_header(headers: &HeaderMap) -> Option<String> {
    let encoded = header_string(headers, REMOTE_CONTROL_SERVER_NAME_HEADER)?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded.as_bytes())
        .ok()?;
    let decoded = String::from_utf8(bytes).ok()?;
    let trimmed = decoded.trim().to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}
