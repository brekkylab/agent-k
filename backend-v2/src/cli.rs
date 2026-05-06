use clap::{Parser, Subcommand};

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
