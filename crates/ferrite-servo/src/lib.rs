// Servo embedding crate — implementation follows in subsequent blocks
pub mod bundle;
pub mod diag;
#[cfg(feature = "servo")]
mod gpu_context;
pub mod permissions;
pub mod session;
pub mod shell;
