//! Durable startup identity. Intent is not permission to replay a key.
use alloc::string::String;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaudeStartup {
    pub instance: String,
    pub session: String,
    pub launch: String,
    pub generation: Option<String>,
    pub halted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaudeStartupKey {
    pub startup: ClaudeStartup,
    pub generation: String,
    pub prompt: String,
    pub attempt: String,
}
