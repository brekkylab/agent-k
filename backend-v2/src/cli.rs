use clap::{Parser, Subcommand};
use uuid::Uuid;

use agent_k_backend::{auth, repository};

#[derive(Parser)]
#[command(name = "agent-k-backend", about = "Agent-K backend server")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Run the HTTP server (default when no subcommand is given)
    Serve,
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

pub async fn run_create_admin(username: String, password: String, display_name: Option<String>) {
    use repository::{NewUser, RepositoryError};

    let repo = repository::create_repository_from_env()
        .await
        .expect("failed to initialise repository");

    let password_hash = match auth::hash_password(&password) {
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
            role: auth::Role::Admin,
            display_name,
            is_active: true,
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
