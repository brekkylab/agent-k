fn main() -> std::io::Result<()> {
    // A re-invoked sandbox boot child boots the microVM from a link-time ctor in
    // ailoy (before this `main`, `_exit`ing on guest shutdown), so the normal
    // server process has nothing to do here.
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(agent_k_backend::run())
}
