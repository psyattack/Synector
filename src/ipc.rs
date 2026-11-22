pub mod client;
pub mod framing;
pub mod overlay_wire;
pub mod protocol;

pub use client::IpcClient;
pub use framing::MAX_PAYLOAD_BYTES;
pub use protocol::{ActiveRuleExecutionState, DaemonState, IpcRequest, IpcResponse};

pub const PIPE_NAME: &str = r"\\.\pipe\Synector_Daemon";
