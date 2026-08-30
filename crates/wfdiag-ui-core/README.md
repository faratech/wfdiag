# wfdiag-ui-core

Framework-neutral event contracts shared by WFDiag backends and native UI
shells. The crate has no Tauri, Reactor, WinUI, or Tokio runtime dependency.

`event_bus` uses two delivery policies:

- Diagnostic results, chat, report, action, quick-scan, and terminal task
  events enter a bounded FIFO. Asynchronous publishers wait for capacity;
  non-blocking publishers get their original event back when the FIFO is full.
- System statistics occupy one fixed latest-value slot. Nonterminal task
  progress has one latest-value slot per `(session_id, task_id)`, bounded by the
  configured capacity. Replacing a value is reported to the publisher.

The UI side calls `UiEventReceiver::drain` from its own thread (for example,
from a WinUI `DispatcherQueueTimer`).
