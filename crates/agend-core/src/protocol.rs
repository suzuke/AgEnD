//! Wire protocol definitions shared by every process (D1, D11).
//!
//! There is exactly one public, versioned client protocol (event stream plus
//! terminal stream) and one versioned holder protocol. Both must stay backward
//! compatible because old holders and old clients keep running across a daemon
//! upgrade. External GUIs consume a JSON schema generated from these types
//! (`xtask`, not implemented yet).
//!
//! Must NOT: perform I/O. Encoding/decoding of in-memory values only.

pub mod client;
pub mod holder;
