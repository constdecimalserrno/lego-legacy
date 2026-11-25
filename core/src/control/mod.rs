//! Composable control plane unifying human play, live agent control, and piped
//! TAS/route tasks over any [`crate::host::Host`] (#308).
//!
//! Strictly outside guest state (Golden Rules 1 & 2, ADR-0001): every action
//! compiles into deterministic [`crate::host::Input`] events delivered at flip
//! boundaries or between-flip input polls, automatically recorded into
//! `#lego-legacy-route 1`.

pub mod bus;
pub mod host;
pub mod stdio;
pub mod timeline;

pub use bus::{
    ActionFeedEntry, AgentState, AgentStatePatch, Command, ControlBus, ControlConfig,
    ControlStatus, ExecMode, FrameDiff, FrameInfoData, FrameSnapshot, InputOwner, Response,
    ResponseData, WaitResult,
};
pub use host::ControlHost;
pub use stdio::StdioBridge;
pub use timeline::{Action, ActionError, ScheduleReport, Timeline};
