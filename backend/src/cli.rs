use agent_k_backend::{authn, repository};
use clap::{Args, Parser, Subcommand, ValueEnum};
use uuid::Uuid;

#[derive(Parser)]
#[command(name = "agent-k-backend", about = "Agent-K backend server")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Run the HTTP server and/or worker loops (default when no subcommand is given)
    Serve(ServeArgs),
    /// Create an admin user (idempotent: errors on duplicate username)
    CreateAdmin {
        #[arg(long)]
        username: String,
        #[arg(long)]
        password: String,
        #[arg(long)]
        display_name: Option<String>,
    },
}

#[derive(Args, Default, Clone)]
pub struct ServeArgs {
    /// Which components to run in this process.
    #[arg(long, value_enum, default_value_t = ServeMode::All)]
    pub mode: ServeMode,
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, ValueEnum)]
pub enum ServeMode {
    /// HTTP API only — no worker / housekeeper / cron ticker.
    Api,
    /// Worker + housekeeper + cron ticker only — no HTTP listener.
    Worker,
    /// Both API and worker loops in the same process (default).
    #[default]
    All,
}

impl ServeMode {
    pub fn runs_api(self) -> bool {
        matches!(self, ServeMode::Api | ServeMode::All)
    }
    pub fn runs_worker(self) -> bool {
        matches!(self, ServeMode::Worker | ServeMode::All)
    }
}

pub async fn run_create_admin(username: String, password: String, display_name: Option<String>) {
    use repository::{NewUser, RepositoryError};

    let repo = repository::create_repository_from_env()
        .await
        .expect("failed to initialise repository");

    let password_hash = match authn::hash_password(&password) {
        Ok(h) => h,
        Err(_) => {
            eprintln!("error: failed to hash password");
            std::process::exit(1);
        }
    };

    let result = repo
        .create_user(NewUser {
            id: Uuid::new_v4(),
            username: username.clone(),
            password_hash,
            role: authn::Role::Admin,
            display_name,
            is_active: true,
            preferred_language: "en".to_string(),
        })
        .await;

    match result {
        Ok(user) => {
            println!("admin user '{}' created (id={})", user.username, user.id);
        }
        Err(RepositoryError::UniqueViolation(_)) => {
            eprintln!("error: username '{}' already exists", username);
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}
