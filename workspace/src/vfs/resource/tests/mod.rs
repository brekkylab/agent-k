//! Tests that need a provider *and* the wrapper it runs behind.
//!
//! Each provider tests itself in its own file, bare. What that cannot reach is the
//! seam: `build_mounts` always wraps a provider in [`CachedResource`], and the wrapper
//! is where a listing feeds a `stat` and that `stat` picks a read's fetch strategy. A
//! mistake that only exists in that hand-off passes every test written against the
//! provider alone — which is how a listing's placeholder length came to be served as a
//! measurement, turning each window of a read into a whole-object fetch.
//!
//! These live inside the crate because the wrapper is `pub(crate)`: an integration test
//! under `workspace/tests/` can reach `WorkspaceFs`, but not the pair directly.
//!
//! [`CachedResource`]: crate::vfs::cache::CachedResource

mod gdrive_mounted;
mod github_mounted;
