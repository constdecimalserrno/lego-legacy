//! Composable control plane unifying human play, live agent control, piped
//! TAS/route tasks, and HTTP/UDS introspection over any [`crate::host::Host`]
//! (#308, #309).
//!
//! Strictly outside guest state (Golden Rules 1 & 2, ADR-0001): every action
//! compiles into deterministic [`crate::host::Input`] events delivered at flip
//! boundaries or between-flip input polls, automatically recorded into
//! `#lego-legacy-route 1`.

pub mod bus;
pub mod host;
pub mod http;
pub mod inspect;
pub mod json;
pub mod observe;
pub mod spectator;
pub mod stdio;
pub mod timeline;
#[cfg(unix)]
pub mod unix;
pub mod ws;

pub use bus::{
    ActionFeedEntry, AgentState, AgentStatePatch, Command, ControlBus, ControlConfig,
    ControlStatus, ExecMode, FrameDiff, FrameInfoData, FrameSnapshot, InputOwner, Response,
    ResponseData, RewindResult, WaitResult,
};
pub use host::{service_flip_boundary, ControlHost};
pub use http::{
    write_session_registry, ExtraRoutes, HttpContext, HttpRequest, HttpResponse, HttpServer,
    SessionRegistryGuard, StreamIo,
};
pub use spectator::SpectatorRoutes;
pub use stdio::StdioBridge;
pub use timeline::{Action, ActionError, ScheduleReport, Timeline};
#[cfg(unix)]
pub use unix::UnixServer;
