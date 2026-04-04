use clap::{Parser, Subcommand};

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
    _permission_mode: Option<String>,
    _max_turns: Option<u64>,
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

    if is_json {
        let session_id = session.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let init_event = serde_json::json!({
            "type": "system",
            "subtype": "init",
            "session_id": session_id,
            "model": model.as_deref().unwrap_or("anthropic/claude-sonnet-4-5"),
        });
        println!("{}", serde_json::to_string(&init_event)?);

        let text_event = serde_json::json!({
            "type": "text",
            "content": format!("Received prompt: {}", prompt),
        });
        println!("{}", serde_json::to_string(&text_event)?);

        let result_event = serde_json::json!({
            "type": "result",
            "subtype": "success",
            "session_id": session_id,
            "usage": {
                "input_tokens": 0,
                "output_tokens": 0,
            },
            "cost_usd": 0.0,
        });
        println!("{}", serde_json::to_string(&result_event)?);
    } else {
        println!("Prompt: {}", prompt);
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
    use std::path::PathBuf;

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
