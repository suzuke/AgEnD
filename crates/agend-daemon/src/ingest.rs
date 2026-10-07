//! Replay durable helper hooks and ACKs, including after their helper exits.
//! Only commit receipts authorize unlink. No content is ever replayed here.
use crate::{claude_bridge::ClaudeBridge, handlers::Context};
use agend_core::protocol::client::{
    ClaudeOperation, ClaudePendingRecord, ClientResponse, MAX_LINE_BYTES, V1_5,
};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read},
    os::{fd::AsRawFd, unix::fs::OpenOptionsExt},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
#[derive(Debug)]
struct Batch {
    lock: File,
    records: Vec<(PathBuf, ClaudePendingRecord)>,
    last: Option<PathBuf>,
}
impl Drop for Batch {
    fn drop(&mut self) {
        unsafe {
            libc::flock(self.lock.as_raw_fd(), libc::LOCK_UN);
        }
    }
}
fn pending(home: &Path, kind: &str, after: Option<PathBuf>) -> io::Result<Option<Batch>> {
    let root = home.join("spool");
    for dir in [&root, &root.join(kind)] {
        match fs::symlink_metadata(dir) {
            Ok(m) if m.is_dir() => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            _ => return Err(io::Error::other("refusing non-directory spool")),
        }
    }
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(root.join("lock"))?;
    if !lock.metadata()?.is_file() {
        return Err(io::Error::other("invalid spool lock"));
    }
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Ok(None);
    }
    let mut batch = Batch {
        lock,
        records: vec![],
        last: None,
    };
    let mut paths = vec![];
    for e in fs::read_dir(root.join(kind))? {
        let e = e?;
        if e.file_name().to_string_lossy().ends_with(".json") {
            paths.push(e.path());
        }
    }
    paths.sort();
    let remaining: Vec<_> = paths
        .iter()
        .filter(|p| after.as_ref().is_none_or(|a| *p > a))
        .cloned()
        .collect();
    // Rotate past retained refusals/corrupt files; they must not starve later
    // valid ACKs. A wrapped pass still preserves each batch's filename order.
    let paths = if remaining.is_empty() {
        paths
    } else {
        remaining
    };
    for path in paths.into_iter().take(32) {
        batch.last = Some(path.clone());
        let result = (|| {
            let f = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
                .open(&path)?;
            if !f.metadata()?.is_file() || f.metadata()?.len() > MAX_LINE_BYTES as u64 {
                return Err(io::Error::other("invalid pending file"));
            }
            let mut bytes = vec![];
            f.take((MAX_LINE_BYTES + 1) as u64)
                .read_to_end(&mut bytes)?;
            if bytes.len() > MAX_LINE_BYTES {
                return Err(io::Error::other("oversized pending file"));
            }
            let mut p: ClaudePendingRecord = serde_json::from_slice(&bytes)?;
            if p.version != 1 {
                return Err(io::Error::other("unsupported pending version"));
            }
            match (kind, &mut p.request.operation) {
                ("hooks", ClaudeOperation::Hook { replayed, .. }) => *replayed = true,
                ("acks", ClaudeOperation::Ack { .. }) => {}
                _ => return Err(io::Error::other("pending record is not a hook or ACK")),
            }
            Ok(p)
        })();
        match result {
            Ok(record) => batch.records.push((path, record)),
            Err(e) => crate::log::line(&format!("Claude pending {} retained: {e}", path.display())),
        }
    }
    Ok(Some(batch))
}
pub(crate) async fn run(home: PathBuf, ctx: Arc<Context>, bridge: Arc<ClaudeBridge>) {
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    let mut cursors = std::collections::BTreeMap::new();
    let mut unknown_after = 0;
    let mut open_unknown_after = 0;
    let mut telegram_unknown_after = String::new();
    loop {
        tick.tick().await;
        match crate::handlers::telegram_attention::refresh(&ctx, &telegram_unknown_after).await {
            Ok(after) => telegram_unknown_after = after,
            Err(error) => crate::log::line(&format!(
                "Telegram delivery attention refresh failed: {error}"
            )),
        }
        match crate::handlers::claude_attention::refresh(&ctx, unknown_after).await {
            Ok(after) => unknown_after = after,
            Err(e) => crate::log::line(&format!("Claude delivery attention refresh failed: {e}")),
        }
        match crate::handlers::opencode_delivery_attention::refresh(&ctx, open_unknown_after).await
        {
            Ok(after) => open_unknown_after = after,
            Err(e) => crate::log::line(&format!("OpenCode delivery attention refresh failed: {e}")),
        }
        if let Err(e) = crate::handlers::opencode_attention::refresh(&ctx).await {
            crate::log::line(&format!(
                "OpenCode permission attention refresh failed: {e}"
            ));
        }
        for kind in ["hooks", "acks"] {
            let dir_home = home.clone();
            let after = cursors.get(kind).cloned();
            let batch =
                match tokio::task::spawn_blocking(move || pending(&dir_home, kind, after)).await {
                    Ok(Ok(Some(batch))) => batch,
                    Ok(Ok(None)) => continue,
                    result => {
                        crate::log::line(&format!("Claude {kind} spool scan failed: {result:?}"));
                        continue;
                    }
                };
            if let Some(last) = &batch.last {
                cursors.insert(kind, last.clone());
            }
            for (path, record) in &batch.records {
                let reply = bridge
                    .handle(
                        &ctx,
                        Some(&record.request.instance_id),
                        V1_5,
                        record.request.clone(),
                    )
                    .await;
                match reply {
                    ClientResponse::Claude { data } if data.committed => {
                        let path = path.clone();
                        match tokio::task::spawn_blocking(move || {
                            fs::remove_file(&path)?;
                            File::open(path.parent().unwrap())?.sync_all()
                        })
                        .await
                        {
                            Ok(Ok(())) => {}
                            result => crate::log::line(&format!(
                                "Claude pending removal failed: {result:?}"
                            )),
                        }
                    }
                    _ => crate::log::line(&format!(
                        "Claude pending {} retained: {reply:?}",
                        path.display()
                    )),
                }
            }
        }
    }
}
