// Keep the pure issue boundary independently testable while the Reactor
// component integration in `main.rs` evolves.
#[path = "../src/issue_support.rs"]
mod issue_support;
