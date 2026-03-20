//! Example: tool call permission approval flow.
//!
//! Demonstrates how to handle all `ServerRequest` variants when the approval
//! policy is set to `OnRequest` (always ask the user):
//!
//! - `CommandExecutionRequestApproval` — approve/deny shell commands
//! - `FileChangeRequestApproval` — approve/deny file writes
//! - `PermissionsRequestApproval` — grant additional permissions (network, fs)
//! - `ToolRequestUserInput` — answer interactive questions from a tool
//! - `McpServerElicitationRequest` — respond to MCP server elicitations
//! - `DynamicToolCall` — handle client-side dynamic tool invocations
//!
//! The event loop receives requests from the app-server, prints details to
//! stdout, reads decisions from stdin, and resolves or rejects accordingly.
//!
//! Usage:
//!   cargo run --example tool_approval -- "list files in the current directory"

use std::collections::HashMap;
use std::env;
use std::io::Write;
use std::sync::Arc;
use std::time::Duration;

use codex_app_server_client::DEFAULT_IN_PROCESS_CHANNEL_CAPACITY;
use codex_app_server_client::InProcessAppServerClient;
use codex_app_server_client::InProcessClientStartArgs;
use codex_app_server_client::InProcessServerEvent;
use codex_app_server_protocol::AskForApproval;
use codex_app_server_protocol::ClientRequest;
use codex_app_server_protocol::CommandExecutionApprovalDecision;
use codex_app_server_protocol::CommandExecutionRequestApprovalResponse;
use codex_app_server_protocol::DynamicToolCallOutputContentItem;
use codex_app_server_protocol::DynamicToolCallResponse;
use codex_app_server_protocol::FileChangeApprovalDecision;
use codex_app_server_protocol::FileChangeRequestApprovalResponse;
use codex_app_server_protocol::GrantedPermissionProfile;
use codex_app_server_protocol::McpServerElicitationAction;
use codex_app_server_protocol::McpServerElicitationRequestResponse;
use codex_app_server_protocol::PermissionGrantScope;
use codex_app_server_protocol::PermissionsRequestApprovalResponse;
use codex_app_server_protocol::RequestId;
use codex_app_server_protocol::SandboxMode;
use codex_app_server_protocol::ServerNotification;
use codex_app_server_protocol::ServerRequest;
use codex_app_server_protocol::ThreadStartParams;
use codex_app_server_protocol::ThreadStartResponse;
use codex_app_server_protocol::ToolRequestUserInputAnswer;
use codex_app_server_protocol::ToolRequestUserInputResponse;
use codex_app_server_protocol::TurnStartParams;
use codex_app_server_protocol::TurnStartResponse;
use codex_app_server_protocol::TurnStatus;
use codex_app_server_protocol::UserInput;
use codex_arg0::Arg0DispatchPaths;
use codex_core::config::ConfigBuilder;
use codex_core::config_loader::CloudRequirementsLoader;
use codex_core::config_loader::LoaderOverrides;
use codex_feedback::CodexFeedback;
use codex_protocol::protocol::SessionSource;

type DynError = Box<dyn std::error::Error + Send + Sync>;

fn main() -> Result<(), DynError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async_main())
}

async fn async_main() -> Result<(), DynError> {
    let prompt = env::args()
        .nth(1)
        .unwrap_or_else(|| "Create a file called hello.txt containing 'Hello World'".to_string());

    let config = Arc::new(ConfigBuilder::default().build().await?);

    let mut client = InProcessAppServerClient::start(InProcessClientStartArgs {
        arg0_paths: Arg0DispatchPaths::default(),
        config,
        cli_overrides: Vec::new(),
        loader_overrides: LoaderOverrides::default(),
        cloud_requirements: CloudRequirementsLoader::default(),
        feedback: CodexFeedback::new(),
        config_warnings: Vec::new(),
        session_source: SessionSource::Cli,
        enable_codex_api_key_env: true,
        client_name: "tool-approval-example".to_string(),
        client_version: env!("CARGO_PKG_VERSION").to_string(),
        experimental_api: true,
        opt_out_notification_methods: Vec::new(),
        channel_capacity: DEFAULT_IN_PROCESS_CHANNEL_CAPACITY,
    })
    .await?;

    // Start a thread with approval_policy = OnRequest so every tool call
    // triggers an approval request back to this client.
    let thread: ThreadStartResponse = client
        .request_typed(ClientRequest::ThreadStart {
            request_id: RequestId::Integer(1),
            params: ThreadStartParams {
                approval_policy: Some(AskForApproval::OnRequest),
                // Use DangerFullAccess to skip Windows sandbox setup refresh.
                // For production use, prefer WorkspaceWrite or ReadOnly after
                // running the sandbox setup (codex-windows-sandbox-setup.exe).
                sandbox: Some(SandboxMode::DangerFullAccess),
                ..ThreadStartParams::default()
            },
        })
        .await?;
    let thread_id = &thread.thread.id;
    println!("[thread] started: {thread_id}");

    // Start a turn with the user prompt.
    let turn: TurnStartResponse = client
        .request_typed(ClientRequest::TurnStart {
            request_id: RequestId::Integer(2),
            params: TurnStartParams {
                thread_id: thread_id.clone(),
                input: vec![UserInput::Text {
                    text: prompt.clone(),
                    text_elements: Vec::new(),
                }],
                ..TurnStartParams::default()
            },
        })
        .await?;
    let turn_id = &turn.turn.id;
    println!("[turn] started: {turn_id}");
    println!("[prompt] {prompt}\n");

    // Event loop: consume events until the turn completes.
    let result = event_loop(&mut client, thread_id, turn_id).await;

    let _ = client.shutdown().await;
    result
}

/// Main event loop that handles approval requests and collects assistant output.
async fn event_loop(
    client: &mut InProcessAppServerClient,
    thread_id: &str,
    turn_id: &str,
) -> Result<(), DynError> {
    let mut assistant_text = String::new();

    loop {
        let event = tokio::time::timeout(Duration::from_secs(120), client.next_event())
            .await
            .map_err(|_| "timed out waiting for event")?
            .ok_or("event stream closed")?;

        match event {
            InProcessServerEvent::Lagged { skipped } => {
                eprintln!("[warn] lagged, skipped {skipped} events");
            }

            // ── 1. Command execution approval ────────────────────────────
            InProcessServerEvent::ServerRequest(
                ServerRequest::CommandExecutionRequestApproval { request_id, params },
            ) => {
                println!("┌─ Command Approval Request ─────────────────────");
                if let Some(cmd) = &params.command {
                    println!("│ command: {cmd}");
                }
                if let Some(cwd) = &params.cwd {
                    println!("│ cwd:     {}", cwd.display());
                }
                if let Some(reason) = &params.reason {
                    println!("│ reason:  {reason}");
                }
                println!("└────────────────────────────────────────────────");

                let decision = ask_command_decision()?;
                let response = CommandExecutionRequestApprovalResponse { decision };
                client
                    .resolve_server_request(request_id, serde_json::to_value(response)?)
                    .await?;
            }

            // ── 2. File change approval ──────────────────────────────────
            InProcessServerEvent::ServerRequest(ServerRequest::FileChangeRequestApproval {
                request_id,
                params,
            }) => {
                println!("┌─ File Change Approval Request ─────────────────");
                if let Some(reason) = &params.reason {
                    println!("│ reason:     {reason}");
                }
                if let Some(root) = &params.grant_root {
                    println!("│ grant_root: {}", root.display());
                }
                println!("└────────────────────────────────────────────────");

                let decision = ask_file_change_decision()?;
                let response = FileChangeRequestApprovalResponse { decision };
                client
                    .resolve_server_request(request_id, serde_json::to_value(response)?)
                    .await?;
            }

            // ── 3. Permissions approval (network / filesystem) ───────────
            InProcessServerEvent::ServerRequest(
                ServerRequest::PermissionsRequestApproval { request_id, params },
            ) => {
                println!("┌─ Permissions Approval Request ─────────────────");
                if let Some(reason) = &params.reason {
                    println!("│ reason:  {reason}");
                }
                if let Some(net) = &params.permissions.network {
                    println!("│ network: enabled={:?}", net.enabled);
                }
                if let Some(fs) = &params.permissions.file_system {
                    println!("│ fs:      {fs:?}");
                }
                println!("└────────────────────────────────────────────────");

                let (granted, scope) = ask_permissions_decision(&params.permissions)?;
                let response = PermissionsRequestApprovalResponse {
                    permissions: granted,
                    scope,
                };
                client
                    .resolve_server_request(request_id, serde_json::to_value(response)?)
                    .await?;
            }

            // ── 4. Tool request user input (interactive questions) ───────
            InProcessServerEvent::ServerRequest(ServerRequest::ToolRequestUserInput {
                request_id,
                params,
            }) => {
                println!("┌─ Tool User Input Request ──────────────────────");
                let mut answers_map = HashMap::new();
                for question in &params.questions {
                    println!("│ [{question_id}] {header}", question_id = question.id, header = question.header);
                    println!("│   {}", question.question);
                    if let Some(options) = &question.options {
                        for (i, opt) in options.iter().enumerate() {
                            println!("│   {i}. {} - {}", opt.label, opt.description);
                        }
                    }
                    print!("│ > ");
                    std::io::stdout().flush()?;
                    let mut input = String::new();
                    std::io::stdin().read_line(&mut input)?;
                    answers_map.insert(
                        question.id.clone(),
                        ToolRequestUserInputAnswer {
                            answers: vec![input.trim().to_string()],
                        },
                    );
                }
                println!("└────────────────────────────────────────────────");

                let response = ToolRequestUserInputResponse { answers: answers_map };
                client
                    .resolve_server_request(request_id, serde_json::to_value(response)?)
                    .await?;
            }

            // ── 5. MCP server elicitation ────────────────────────────────
            InProcessServerEvent::ServerRequest(
                ServerRequest::McpServerElicitationRequest { request_id, params },
            ) => {
                println!("┌─ MCP Elicitation Request ──────────────────────");
                println!("│ server: {}", params.server_name);
                match &params.request {
                    codex_app_server_protocol::McpServerElicitationRequest::Form {
                        message, ..
                    } => {
                        println!("│ mode:    form");
                        println!("│ message: {message}");
                    }
                    codex_app_server_protocol::McpServerElicitationRequest::Url {
                        message,
                        url,
                        ..
                    } => {
                        println!("│ mode:    url");
                        println!("│ message: {message}");
                        println!("│ url:     {url}");
                    }
                }
                println!("└────────────────────────────────────────────────");

                let action = ask_elicitation_decision()?;
                let response = McpServerElicitationRequestResponse {
                    action,
                    content: None,
                    meta: None,
                };
                client
                    .resolve_server_request(request_id, serde_json::to_value(response)?)
                    .await?;
            }

            // ── 6. Dynamic tool call (client-side tool execution) ────────
            InProcessServerEvent::ServerRequest(ServerRequest::DynamicToolCall {
                request_id,
                params,
            }) => {
                println!("┌─ Dynamic Tool Call ────────────────────────────");
                println!("│ tool: {}", params.tool);
                println!("│ args: {}", params.arguments);
                println!("└────────────────────────────────────────────────");

                // This example does not implement any dynamic tools, so we
                // return a failure response indicating the tool is unknown.
                let response = DynamicToolCallResponse {
                    content_items: vec![DynamicToolCallOutputContentItem::InputText {
                        text: format!("tool '{}' is not implemented in this example", params.tool),
                    }],
                    success: false,
                };
                client
                    .resolve_server_request(request_id, serde_json::to_value(response)?)
                    .await?;
            }

            // ── 7. ChatGPT auth token refresh ────────────────────────────
            InProcessServerEvent::ServerRequest(ServerRequest::ChatgptAuthTokensRefresh {
                request_id,
                ..
            }) => {
                eprintln!("[info] auth token refresh requested, rejecting (not implemented)");
                client
                    .reject_server_request(
                        request_id,
                        codex_app_server_protocol::JSONRPCErrorError {
                            code: -32601,
                            message: "auth token refresh not implemented in this example"
                                .to_string(),
                            data: None,
                        },
                    )
                    .await?;
            }

            // ── Streaming assistant text ─────────────────────────────────
            InProcessServerEvent::ServerNotification(ServerNotification::AgentMessageDelta(
                notification,
            )) if notification.thread_id == thread_id && notification.turn_id == turn_id => {
                print!("{}", notification.delta);
                let _ = std::io::stdout().flush();
                assistant_text.push_str(&notification.delta);
            }

            // ── Turn completed ───────────────────────────────────────────
            InProcessServerEvent::ServerNotification(ServerNotification::TurnCompleted(
                notification,
            )) if notification.thread_id == thread_id && notification.turn.id == turn_id => {
                println!();
                match notification.turn.status {
                    TurnStatus::Completed => {
                        println!("\n[done] turn completed successfully");
                        return Ok(());
                    }
                    TurnStatus::Failed => {
                        let msg = notification
                            .turn
                            .error
                            .map(|e| e.message)
                            .unwrap_or_else(|| "unknown error".to_string());
                        return Err(msg.into());
                    }
                    status => {
                        return Err(format!("unexpected turn status: {status:?}").into());
                    }
                }
            }

            // ── Turn error ───────────────────────────────────────────────
            InProcessServerEvent::ServerNotification(ServerNotification::Error(notification))
                if notification.thread_id == thread_id && notification.turn_id == turn_id =>
            {
                return Err(notification.error.message.into());
            }

            // ── Ignore other notifications ───────────────────────────────
            _ => {}
        }
    }
}

// ─── Stdin helpers ───────────────────────────────────────────────────────────

fn read_line_trimmed() -> Result<String, DynError> {
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    Ok(input.trim().to_lowercase())
}

/// Prompt the user for a command execution approval decision.
fn ask_command_decision() -> Result<CommandExecutionApprovalDecision, DynError> {
    print!("Approve? [y]es / [s]ession / [n]o / [c]ancel: ");
    std::io::stdout().flush()?;
    match read_line_trimmed()?.as_str() {
        "y" | "yes" => Ok(CommandExecutionApprovalDecision::Accept),
        "s" | "session" => Ok(CommandExecutionApprovalDecision::AcceptForSession),
        "n" | "no" => Ok(CommandExecutionApprovalDecision::Decline),
        "c" | "cancel" => Ok(CommandExecutionApprovalDecision::Cancel),
        _ => {
            println!("(unrecognized, defaulting to Decline)");
            Ok(CommandExecutionApprovalDecision::Decline)
        }
    }
}

/// Prompt the user for a file change approval decision.
fn ask_file_change_decision() -> Result<FileChangeApprovalDecision, DynError> {
    print!("Allow file change? [y]es / [s]ession / [n]o / [c]ancel: ");
    std::io::stdout().flush()?;
    match read_line_trimmed()?.as_str() {
        "y" | "yes" => Ok(FileChangeApprovalDecision::Accept),
        "s" | "session" => Ok(FileChangeApprovalDecision::AcceptForSession),
        "n" | "no" => Ok(FileChangeApprovalDecision::Decline),
        "c" | "cancel" => Ok(FileChangeApprovalDecision::Cancel),
        _ => {
            println!("(unrecognized, defaulting to Decline)");
            Ok(FileChangeApprovalDecision::Decline)
        }
    }
}

/// Prompt the user for a permissions approval decision.
///
/// On accept, echoes back the requested permissions. On deny, returns an empty
/// profile so the server knows nothing was granted.
fn ask_permissions_decision(
    requested: &codex_app_server_protocol::RequestPermissionProfile,
) -> Result<(GrantedPermissionProfile, PermissionGrantScope), DynError> {
    print!("Grant permissions? [y]es / [s]ession / [n]o: ");
    std::io::stdout().flush()?;
    match read_line_trimmed()?.as_str() {
        "y" | "yes" => Ok((
            GrantedPermissionProfile {
                network: requested.network.clone(),
                file_system: requested.file_system.clone(),
            },
            PermissionGrantScope::Turn,
        )),
        "s" | "session" => Ok((
            GrantedPermissionProfile {
                network: requested.network.clone(),
                file_system: requested.file_system.clone(),
            },
            PermissionGrantScope::Session,
        )),
        _ => {
            println!("(denied)");
            Ok((GrantedPermissionProfile::default(), PermissionGrantScope::Turn))
        }
    }
}

/// Prompt the user for an MCP elicitation decision.
fn ask_elicitation_decision() -> Result<McpServerElicitationAction, DynError> {
    print!("MCP elicitation: [a]ccept / [d]ecline / [c]ancel: ");
    std::io::stdout().flush()?;
    match read_line_trimmed()?.as_str() {
        "a" | "accept" => Ok(McpServerElicitationAction::Accept),
        "d" | "decline" => Ok(McpServerElicitationAction::Decline),
        "c" | "cancel" => Ok(McpServerElicitationAction::Cancel),
        _ => {
            println!("(unrecognized, defaulting to Decline)");
            Ok(McpServerElicitationAction::Decline)
        }
    }
}
