//! UI-framework-neutral generation of `WFDiag` export payloads.
//!
//! Rendering is pure and delivery-independent. [`ExportRuntime`] moves the
//! same work to a dedicated thread so a native UI can enqueue generation
//! without parsing or formatting diagnostic output on its dispatcher thread.

#![deny(unsafe_code)]

mod renderer;
mod runtime;

pub use renderer::{
    EmailPayload, ExportMetadata, ExportPayload, ExportRequestKind, ExportTask, ReportFormat,
    SupportPackagePayload, format_json_value, render_email, render_email_compose_uri,
    render_forum_clipboard, render_report, render_saved_report, render_support_package,
    render_windows_forum_post,
};
pub use runtime::{ExportCompleted, ExportError, ExportRequest, ExportRuntime};
pub use wfdiag_native_issues::TaskResult;
