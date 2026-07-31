fn main() -> std::io::Result<()> {
    // If this process was re-invoked as a sandbox boot child, boot the microVM
    // here and never return (it `_exit`s on guest shutdown). Returns immediately
    // in the normal server process. Must run before the Tokio runtime starts.
    ailoy::runenv::boot_if_requested();

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(agent_k_backend::run())
}
