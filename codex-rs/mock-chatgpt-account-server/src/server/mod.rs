mod args;
mod config;
mod frontend;
mod responses;
mod routes;
mod state;

use std::net::SocketAddr;

use anyhow::Context;
use anyhow::Result;

pub use args::MockServerArgs;
use config::load_login_ui_config;
use state::AppState;

pub async fn run(args: MockServerArgs) -> Result<()> {
    let bind_addr = resolve_bind_addr(&args).await?;
    let login_ui_config = load_login_ui_config(args.social_login_config.as_deref())?;
    let social_login_providers = login_ui_config
        .social_login_providers()
        .iter()
        .map(|provider| provider.id.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let state = AppState::new(args.clone(), login_ui_config);
    let login_username = args
        .login_username
        .clone()
        .unwrap_or_else(|| args.email.clone());

    println!(
        "Mock account server listening on http://{}:{}",
        args.host, args.port
    );
    println!("OAuth issuer: http://{}:{}", args.host, args.port);
    println!("Browser login: {login_username} / {}", args.login_password);
    if !social_login_providers.is_empty() {
        println!("Shortcut logins: {social_login_providers}");
    }
    println!(
        "Models endpoints: http://{}:{}/models, /v1/models, and /backend-api/codex/models",
        args.host, args.port
    );
    println!(
        "Responses endpoint: http://{}:{}/backend-api/codex/responses",
        args.host, args.port
    );
    println!(
        "ChatGPT backend base URL: http://{}:{}/backend-api",
        args.host, args.port
    );
    println!(
        "Device auth page: http://{}:{}/codex/device",
        args.host, args.port
    );

    warp::serve(routes::routes(state)).run(bind_addr).await;
    Ok(())
}

async fn resolve_bind_addr(args: &MockServerArgs) -> Result<SocketAddr> {
    tokio::net::lookup_host((args.host.as_str(), args.port))
        .await?
        .next()
        .with_context(|| {
            format!(
                "could not resolve bind address for {}:{}",
                args.host, args.port
            )
        })
}
