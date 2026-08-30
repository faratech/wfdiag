//! Re-export of the shared provider-call configuration. The definition lives
//! in the provider crate (the lowest layer); the chat engine, the included
//! compat client, and both shells all see the same type through here.

pub use wfdiag_native_ai_provider::ResolvedProviderConfig;
