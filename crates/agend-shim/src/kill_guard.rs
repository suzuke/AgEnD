//! Guards `kill`, `killall` and `pkill` so an agent cannot kill processes it
//! does not own (kept from v1's `agend-git` kill guard).
//!
//! Must NOT: guard anything but these three tools.
