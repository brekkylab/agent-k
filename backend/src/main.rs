#[tokio::main]
async fn main() -> std::io::Result<()> {
    agent_k_backend::run().await
}
