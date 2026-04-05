use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Parser)]
#[command(
    name = "kn-code",
    version,
    about = "A high-performance, headless-first AI coding agent"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the API server
    Serve {
        #[arg(long, default_value = "3200")]
        port: u16,
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
    },
    /// Run a single prompt
    Run {
        /// The prompt to run
        prompt: Option<String>,
        #[arg(long)]
        format: Option<String>,
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        model: Option<String>,
        #[arg(long)]
        variant: Option<String>,
        #[arg(long)]
        permission_mode: Option<String>,
        #[arg(long)]
        max_turns: Option<u64>,
        #[arg(long)]
        print: Option<String>,
    },
    /// List available models
    Models,
    /// Manage authentication
    Auth {
        #[command(subcommand)]
        command: AuthCommands,
    },
    /// Manage sessions
    Session {
        #[command(subcommand)]
        command: Option<SessionCommands>,
    },
}

#[derive(Subcommand)]
enum AuthCommands {
    /// Login to a provider
    Login {
        #[arg()]
        provider: String,
    },
    /// Logout from a provider
    Logout {
        #[arg()]
        provider: String,
    },
    /// Show auth status
    Status,
}

#[derive(Subcommand)]
enum SessionCommands {
    /// List sessions
    List,
    /// Show session details
    Show { session_id: String },
    /// Delete a session
    Delete { session_id: String },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Serve { port, host }) => cmd_serve(port, host).await,
        Some(Commands::Run {
            prompt,
            format,
            session,
            model,
            variant,
            permission_mode,
            max_turns,
            print,
        }) => {
            cmd_run(
                prompt,
                format,
                session,
                model,
                variant,
                permission_mode,
                max_turns,
                print,
            )
            .await
        }
        Some(Commands::Models) => cmd_models().await,
        Some(Commands::Auth { command }) => cmd_auth(command).await,
        Some(Commands::Session { command }) => cmd_session(command).await,
        None => {
            println!("kn-code v{}", env!("CARGO_PKG_VERSION"));
            println!("Usage: kn-code <command>");
            println!();
            println!("Commands:");
            println!("  serve    Start the API server");
            println!("  run      Run a single prompt");
            println!("  models   List available models");
            println!("  auth     Manage authentication");
            println!("  session  Manage sessions");
            Ok(())
        }
    }
}

async fn cmd_serve(port: u16, host: String) -> anyhow::Result<()> {
    use kn_code_server::server::{Server, ServerConfig};

    let config = ServerConfig {
        host,
        port,
        ..Default::default()
    };

    let server = Server::new(config);
    server.start().await
}

#[allow(clippy::too_many_arguments)]
async fn cmd_run(
    prompt: Option<String>,
    format: Option<String>,
    session: Option<String>,
    model: Option<String>,
    _variant: Option<String>,
    permission_mode: Option<String>,
    max_turns: Option<u64>,
    print: Option<String>,
) -> anyhow::Result<()> {
    let prompt = match (prompt, print) {
        (Some(p), _) => p,
        (None, Some(_)) => {
            let mut input = String::new();
            std::io::stdin().read_line(&mut input)?;
            input
        }
        (None, None) => {
            anyhow::bail!("No prompt provided. Use --print - to read from stdin.");
        }
    };

    let is_json = format.as_deref() == Some("json");
    let cwd = std::env::current_dir().unwrap_or_default();
    let model_str = model
        .as_deref()
        .unwrap_or("anthropic/claude-sonnet-4-5")
        .to_string();

    let session_store = Arc::new(kn_code_session::SessionStore::new(
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join(".kn-code")
            .join("sessions"),
    ));

    let session_id = if let Some(id) = session {
        id
    } else {
        let record = session_store
            .create_session(cwd.clone(), model_str.clone())
            .await?;
        record.id
    };

    let message =
        kn_code_session::messages::Message::User(kn_code_session::messages::UserMessage {
            id: uuid::Uuid::new_v4().to_string(),
            content: vec![kn_code_session::messages::ContentBlock::Text(
                prompt.clone(),
            )],
            timestamp: chrono::Utc::now(),
        });

    session_store.append_message(&session_id, &message).await?;

    let token_store = Arc::new(kn_code_auth::FileTokenStore::new(
        kn_code_config::Settings::config_dir().join("tokens.enc"),
    ));

    let tools: Vec<Arc<dyn kn_code_tools::traits::Tool>> = vec![
        Arc::new(kn_code_tools::bash::BashTool::default()),
        Arc::new(kn_code_tools::file_read::FileReadTool),
        Arc::new(kn_code_tools::file_write::FileWriteTool),
        Arc::new(kn_code_tools::file_edit::FileEditTool),
        Arc::new(kn_code_tools::glob::GlobTool),
        Arc::new(kn_code_tools::grep::GrepTool),
        Arc::new(kn_code_tools::web_fetch::WebFetchTool),
    ];

    let perm_mode = match permission_mode.as_deref() {
        Some("auto") => kn_code_permissions::rules::PermissionMode::Auto,
        Some("ask") => kn_code_permissions::rules::PermissionMode::Ask,
        _ => kn_code_permissions::rules::PermissionMode::BypassPermissions,
    };

    let Some((provider, model_info)) = kn_code_providers::resolve_provider(&model_str) else {
        anyhow::bail!(
            "Unknown provider prefix in model '{}'. Supported: anthropic/, openai/, github_copilot/",
            model_str
        );
    };
    let runner = kn_code_session::runner::AgentRunner {
        session_store: session_store.clone(),
        token_store,
        provider,
        tools,
        permission_mode: perm_mode,
        max_turns: max_turns.unwrap_or(50),
        cwd,
        model_info,
        cancellation_token: None,
    };

    if is_json {
        let init_event = serde_json::json!({
            "type": "system",
            "subtype": "init",
            "session_id": session_id,
            "model": model_str,
        });
        println!("{}", serde_json::to_string(&init_event)?);
    } else {
        println!("Starting agent session: {}", session_id);
        println!("Model: {}", model_str);
        println!("---");
    }

    match runner.run(&session_id).await {
        Ok(result) => {
            if is_json {
                let result_event = serde_json::json!({
                    "type": "result",
                    "subtype": result.stop_reason,
                    "session_id": result.session_id,
                    "turns_completed": result.turns_completed,
                    "usage": {
                        "input_tokens": result.input_tokens,
                        "output_tokens": result.output_tokens,
                    },
                    "cost_usd": result.cost_usd,
                });
                println!("{}", serde_json::to_string(&result_event)?);
            } else {
                println!("---");
                println!("Session completed: {}", session_id);
                println!("Turns: {}", result.turns_completed);
                println!("Stop reason: {}", result.stop_reason);
                println!(
                    "Tokens: {} in, {} out",
                    result.input_tokens, result.output_tokens
                );
            }
        }
        Err(e) => {
            if is_json {
                let error_event = serde_json::json!({
                    "type": "error",
                    "session_id": session_id,
                    "error": e.to_string(),
                });
                println!("{}", serde_json::to_string(&error_event)?);
            } else {
                eprintln!("Agent run failed: {}", e);
            }
            std::process::exit(1);
        }
    }

    Ok(())
}

async fn cmd_models() -> anyhow::Result<()> {
    let models = vec![
        "anthropic/claude-sonnet-4-5",
        "anthropic/claude-opus-4-5",
        "anthropic/claude-haiku-4-5",
        "github_copilot/gpt-4o",
        "github_copilot/claude-sonnet-4",
        "openai/gpt-4o",
        "openai/o1",
    ];

    for model in models {
        println!("{}", model);
    }

    Ok(())
}

async fn cmd_auth(command: AuthCommands) -> anyhow::Result<()> {
    use kn_code_auth::{ApiKeyAuth, FileTokenStore, TokenStore};

    let store_dir = kn_code_config::Settings::config_dir();
    let store_path = store_dir.join("tokens.enc");
    let store = FileTokenStore::new(store_path);

    match command {
        AuthCommands::Login { provider } => {
            let api_key = std::env::var(format!(
                "{}_API_KEY",
                provider.to_uppercase().replace('-', "_")
            ));
            match api_key {
                Ok(key) => {
                    let auth = ApiKeyAuth::new(key)?;
                    let creds = kn_code_auth::Credentials {
                        provider_id: provider.clone(),
                        auth_type: kn_code_auth::AuthType::ApiKey,
                        api_key: Some(auth.api_key),
                        access_token: None,
                        refresh_token: None,
                        expires_at: None,
                        account_uuid: None,
                        user_email: None,
                        organization_uuid: None,
                    };
                    store.store(&provider, &creds).await?;
                    println!("Logged in to {}", provider);
                }
                Err(std::env::VarError::NotPresent) => {
                    anyhow::bail!(
                        "Set {}_API_KEY environment variable and try again.",
                        provider.to_uppercase().replace('-', "_")
                    );
                }
                Err(e) => {
                    anyhow::bail!("Failed to read API key from environment: {}", e);
                }
            }
        }
        AuthCommands::Logout { provider } => {
            store.delete(&provider).await?;
            println!("Logged out from {}", provider);
        }
        AuthCommands::Status => {
            let providers = store.list_providers().await?;
            if providers.is_empty() {
                println!("Not authenticated with any providers.");
            } else {
                println!("Authenticated providers:");
                for p in providers {
                    println!("  - {}", p);
                }
            }
        }
    }
    Ok(())
}

async fn cmd_session(command: Option<SessionCommands>) -> anyhow::Result<()> {
    use kn_code_session::SessionStore;

    let store_dir = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join(".kn-code")
        .join("sessions");
    let store = SessionStore::new(store_dir);

    match command {
        Some(SessionCommands::List) => {
            if !store.base_dir.exists() {
                println!("No sessions found.");
                return Ok(());
            }

            let mut entries = tokio::fs::read_dir(&store.base_dir).await?;
            let mut found = false;
            while let Some(entry) = entries.next_entry().await? {
                if entry.file_type().await?.is_dir() {
                    let session_id = entry.file_name().to_string_lossy().to_string();
                    if let Ok(Some(record)) = store.load_session(&session_id).await {
                        println!(
                            "{} | {} | {} | turns: {} | ${:.4}",
                            record.id,
                            record.model,
                            record.state,
                            record.turns_completed,
                            record.cost_usd,
                        );
                        found = true;
                    }
                }
            }
            if !found {
                println!("No sessions found.");
            }
        }
        Some(SessionCommands::Show { session_id }) => {
            match store.load_session(&session_id).await? {
                Some(record) => {
                    println!("Session: {}", record.id);
                    println!("Model: {}", record.model);
                    println!("State: {}", record.state);
                    println!("CWD: {}", record.cwd.display());
                    println!("Turns: {}", record.turns_completed);
                    println!("Cost: ${:.4}", record.cost_usd);
                    println!("Created: {}", record.created_at);
                    println!("Updated: {}", record.updated_at);
                }
                None => {
                    anyhow::bail!("Session not found: {}", session_id);
                }
            }
        }
        Some(SessionCommands::Delete { session_id }) => {
            let dir = store.session_dir(&session_id);
            if dir.exists() {
                tokio::fs::remove_dir_all(&dir).await?;
                println!("Deleted session: {}", session_id);
            } else {
                anyhow::bail!("Session not found: {}", session_id);
            }
        }
        None => {
            println!("Usage: kn-code session <list|show|delete>");
        }
    }
    Ok(())
}
