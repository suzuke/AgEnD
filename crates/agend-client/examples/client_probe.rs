//! `client_probe`: operator actions through `agend-client` until gate 9 has
//! real commands. Not a production command.
//!
//!   resolve <attention-id> <action>   e.g. `resolve instance-failed:g8-2 retry`
//!
//! The socket is `$AGEND_HOME/run/daemon.sock`; `AGEND_INSTANCE`, when set,
//! makes the caller that agent (the daemon then refuses operator requests).

use std::path::Path;
use std::process::ExitCode;

use agend_client::Client;
use agend_core::protocol::client::{AttentionAction, DAEMON_SOCKET};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [command, attention_id, action] = args.as_slice() else {
        eprintln!("usage: client_probe resolve <attention-id> <action>");
        return ExitCode::from(2);
    };
    if command != "resolve" {
        eprintln!("usage: client_probe resolve <attention-id> <action>");
        return ExitCode::from(2);
    }
    let action: AttentionAction = match serde_json::from_value(serde_json::json!(action)) {
        Ok(action) => action,
        Err(e) => {
            eprintln!("unknown action {action}: {e}");
            return ExitCode::from(2);
        }
    };
    let Some(home) = std::env::var_os("AGEND_HOME").filter(|h| Path::new(h).is_absolute()) else {
        eprintln!("AGEND_HOME must be set to an absolute path");
        return ExitCode::from(1);
    };
    let socket = Path::new(&home).join(DAEMON_SOCKET);
    let caller = std::env::var("AGEND_INSTANCE")
        .ok()
        .filter(|id| !id.is_empty());
    let resolved = Client::connect(&socket, caller)
        .and_then(|mut c| c.resolve_attention(attention_id, action));
    match resolved {
        Ok(()) => {
            println!("resolved");
            ExitCode::SUCCESS
        }
        Err(e) => {
            println!("{e}");
            ExitCode::from(1)
        }
    }
}
