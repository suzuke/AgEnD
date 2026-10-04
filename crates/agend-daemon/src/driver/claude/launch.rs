//! D40 P2/P3: push-only launch configuration and fail-closed file ownership.
//! A file published before its ownership transaction commits is deliberately
//! treated as foreign after a crash. Never infer ownership from its contents.

use crate::store::{Instance, SqliteStore, StoreError};
use agend_core::model::Backend;
use rusqlite::OptionalExtension;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

const MAX_FILE: u64 = 1024 * 1024;
const EVENTS: &[&str] = &[
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "Stop",
    "SessionEnd",
];

pub fn applies(instance: &Instance) -> bool {
    instance.backend == Backend::Claude && instance.delivery == "push"
}

pub fn settings_path(home: &Path, id: &str) -> PathBuf {
    home.join("claude").join(id).join("settings.json")
}

/// No shell parses the launch arguments. Hooks alone require shell quoting.
pub fn args(home: &Path, instance: &Instance) -> Result<Vec<String>, String> {
    crate::store::instances::validate_id(&instance.id)?;
    let reserved = [
        "--settings",
        "--setting-sources",
        "--permission-mode",
        "--channels",
        "--dangerously-load-development-channels",
        "--session-id",
        "--resume",
        "--dangerously-skip-permissions",
        "--allow-dangerously-skip-permissions",
        "--mcp-config",
        "--strict-mcp-config",
        "--print",
        "-p",
    ];
    if let Some(arg) = instance.args.iter().find(|arg| {
        reserved
            .iter()
            .any(|flag| *arg == flag || arg.starts_with(&format!("{flag}=")))
    }) {
        return Err(format!(
            "Claude push launch owns argument {arg}; remove it from instance args"
        ));
    }
    let mut args = instance.args.clone();
    args.extend([
        "--setting-sources".into(),
        "project,local".into(),
        "--settings".into(),
        settings_path(home, &instance.id).display().to_string(),
        "--permission-mode".into(),
        "bypassPermissions".into(),
        "--dangerously-load-development-channels".into(),
        "server:agend".into(),
    ]);
    Ok(args)
}

fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn documents(home: &Path, exe: &Path, instance: &Instance) -> Vec<(PathBuf, Vec<u8>)> {
    let mut hooks = serde_json::Map::new();
    for event in EVENTS {
        hooks.insert((*event).into(), json!([{"hooks":[{
            "type":"command", "command":format!("{} hook {}", quote(&exe.display().to_string()), event),
            "timeout":10
        }]}]));
    }
    let settings = json!({"permissions":{"defaultMode":"bypassPermissions"},
        "enabledMcpjsonServers":["agend"], "hooks":hooks});
    let mcp = json!({"mcpServers":{"agend":{
        "command":exe.display().to_string(), "args":["channel","--instance",instance.id],
        "env":{"AGEND_HOME":home.display().to_string(),"AGEND_INSTANCE":instance.id}
    }}});
    let workspace = Path::new(&instance.working_directory);
    vec![
        (settings_path(home, &instance.id), serde_json::to_vec_pretty(&settings).expect("JSON value")),
        (workspace.join(".mcp.json"), serde_json::to_vec_pretty(&mcp).expect("JSON value")),
        (workspace.join("CLAUDE.md"), b"# AgEnD team messages\n\nThe agend channel and Stop hook deliver messages from the user's own team.\nFor every delivered message, call mcp__agend__agend_ack with its message_id,\ndelivery_id and session_id before starting work. Acknowledge every message\nin a batch; receiving or acknowledging a message does not complete its task.\nUse agend status to read the current work or review ticket and agend done /\nagend review to report the result. Run repository commands through the PATH\nshims. Never merge or approve a pull request directly.\n".to_vec()),
    ]
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn read(path: &Path) -> Result<Option<Vec<u8>>, String> {
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    if !meta.is_file() || meta.nlink() != 1 || meta.len() > MAX_FILE {
        return Err(format!(
            "{} is not a bounded, unlinked regular file; preserved",
            path.display()
        ));
    }
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let mut bytes = Vec::new();
    file.take(MAX_FILE + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_FILE {
        return Err(format!("{} grew beyond the size limit", path.display()));
    }
    Ok(Some(bytes))
}

fn directory(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => Ok(()),
        Ok(_) => Err(format!(
            "{} is not a regular directory; preserved",
            path.display()
        )),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => std::fs::DirBuilder::new()
            .mode(0o700)
            .create(path)
            .map_err(|e| format!("{}: {e}", path.display())),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// Only the supervisor calls this, before Spawn, for Claude push instances.
/// All destinations are checked before the first file is published.
pub async fn prepare(
    store: &SqliteStore,
    home: &Path,
    exe: &Path,
    instance: &Instance,
) -> Result<(), String> {
    if !applies(instance) {
        return Ok(());
    }
    args(home, instance)?;
    let home = home.canonicalize().map_err(|e| e.to_string())?;
    let exe = exe.canonicalize().map_err(|e| e.to_string())?;
    let mut instance = instance.clone();
    instance.working_directory = Path::new(&instance.working_directory)
        .canonicalize()
        .map_err(|e| e.to_string())?
        .display()
        .to_string();
    directory(&home.join("claude"))?;
    directory(&home.join("claude").join(&instance.id))?;
    let docs = documents(&home, &exe, &instance);
    let mut changes = Vec::new();
    for (path, bytes) in docs {
        let key = path.display().to_string();
        let owned: Option<(String, String)> = store
            .call(move |conn| {
                conn.query_row(
                    "SELECT instance_id, sha256 FROM claude_owned_files WHERE path=?1",
                    [key],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()
                .map_err(StoreError::from)
            })
            .await
            .map_err(|e| e.to_string())?;
        let current = read(&path)?;
        if owned.as_ref().is_some_and(|(id, _)| id != &instance.id)
            || current.as_ref().is_some_and(|old| {
                !owned
                    .as_ref()
                    .is_some_and(|(id, hash)| id == &instance.id && *hash == digest(old))
            })
        {
            return Err(format!(
                "{} exists and is foreign or modified; preserved; instance startup refused",
                path.display()
            ));
        }
        changes.push((path, bytes, current));
    }
    for (path, bytes, before) in changes {
        let key = path.display().to_string();
        let hash = digest(&bytes);
        if before.as_deref() != Some(bytes.as_slice()) {
            let target = path.clone();
            tokio::task::spawn_blocking(move || publish(&target, &bytes, before.as_deref()))
                .await
                .map_err(|e| e.to_string())??;
        }
        let owner = instance.id.clone();
        store.call(move |conn| {
            conn.execute("INSERT INTO claude_owned_files(path,instance_id,sha256) VALUES(?1,?2,?3) \
                ON CONFLICT(path) DO UPDATE SET sha256=excluded.sha256 WHERE instance_id=excluded.instance_id",
                rusqlite::params![key,owner,hash])?;
            Ok(())
        }).await.map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn publish(path: &Path, bytes: &[u8], before: Option<&[u8]>) -> Result<(), String> {
    let parent = path.parent().ok_or("file has no parent")?;
    let temp = parent.join(format!(
        ".agend-settings-{}",
        crate::store::instances::new_session_id().map_err(|e| e.to_string())?
    ));
    let result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        if read(path).map_err(std::io::Error::other)?.as_deref() != before {
            return Err(std::io::Error::other("destination changed; preserved"));
        }
        if before.is_none() {
            fs::hard_link(&temp, path)?;
        } else {
            fs::rename(&temp, path)?;
        }
        // Remove the second link before the ownership hash becomes visible.
        if temp.exists() {
            fs::remove_file(&temp)?;
        }
        File::open(parent)?.sync_all()
    })();
    let _ = fs::remove_file(&temp);
    result.map_err(|e| format!("{}: {e}; startup refused", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::InstanceStatus;
    use agend_testkit::tempdir::TempDir;
    use std::os::unix::fs::symlink;

    fn instance(home: &Path) -> Instance {
        let workspace = home.join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        Instance {
            id: "g12-launch".into(),
            backend: Backend::Claude,
            program: "claude".into(),
            args: vec!["--model".into(), "haiku".into()],
            working_directory: workspace.display().to_string(),
            session_id: Some("11111111-1111-4111-8111-111111111111".into()),
            status: InstanceStatus::New,
            session_started: false,
            agent_pid: None,
            legacy_no_thread: false,
            delivery: "push".into(),
        }
    }
    async fn prepare_test(
        store: &SqliteStore,
        home: &Path,
        instance: &Instance,
    ) -> Result<(), String> {
        prepare(store, home, &std::env::current_exe().unwrap(), instance).await
    }

    #[tokio::test]
    async fn push_configuration_is_native_json_with_every_hook_and_explicit_ack() {
        let dir = TempDir::new("g12-launch-json").unwrap();
        let store = SqliteStore::open(dir.path(), 1).unwrap();
        let inst = instance(dir.path());
        prepare_test(&store, dir.path(), &inst).await.unwrap();
        let settings: serde_json::Value =
            serde_json::from_slice(&fs::read(settings_path(dir.path(), &inst.id)).unwrap())
                .unwrap();
        assert_eq!(
            settings["permissions"],
            json!({"defaultMode":"bypassPermissions"})
        );
        assert_eq!(settings["enabledMcpjsonServers"], json!(["agend"]));
        for event in EVENTS {
            assert_eq!(settings["hooks"][event][0]["hooks"][0]["timeout"], 10);
            assert!(
                settings["hooks"][event][0]["hooks"][0]["command"]
                    .as_str()
                    .unwrap()
                    .ends_with(&format!(" hook {event}"))
            );
        }
        let mcp: serde_json::Value = serde_json::from_slice(
            &fs::read(Path::new(&inst.working_directory).join(".mcp.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            mcp["mcpServers"]["agend"]["args"],
            json!(["channel", "--instance", inst.id])
        );
        assert_eq!(
            mcp["mcpServers"]["agend"]["env"]["AGEND_HOME"],
            dir.path().canonicalize().unwrap().display().to_string()
        );
        let instructions =
            fs::read_to_string(Path::new(&inst.working_directory).join("CLAUDE.md")).unwrap();
        assert!(instructions.contains("mcp__agend__agend_ack"));
        assert!(instructions.contains("before starting work"));
        let launched = crate::supervisor::launch(dir.path(), &inst, true).unwrap();
        assert!(
            launched
                .args
                .windows(2)
                .any(|a| a == ["--setting-sources", "project,local"])
        );
        assert!(
            launched
                .args
                .windows(2)
                .any(|a| a == ["--permission-mode", "bypassPermissions"])
        );
        assert_eq!(
            launched.args[launched.args.len() - 2..],
            ["--resume", inst.session_id.as_ref().unwrap()]
        );
        assert!(
            !launched
                .args
                .iter()
                .any(|a| a.contains("CLAUDE_CONFIG_DIR"))
        );
    }

    #[tokio::test]
    async fn foreign_or_modified_files_are_preserved_before_any_other_file_is_written() {
        for name in ["CLAUDE.md", ".mcp.json"] {
            let dir = TempDir::new("g12-launch-foreign").unwrap();
            let store = SqliteStore::open(dir.path(), 1).unwrap();
            let inst = instance(dir.path());
            let path = Path::new(&inst.working_directory).join(name);
            fs::write(&path, b"user content").unwrap();
            let error = prepare_test(&store, dir.path(), &inst).await.unwrap_err();
            assert!(error.contains(name), "{error}");
            assert_eq!(fs::read(&path).unwrap(), b"user content");
            assert!(!settings_path(dir.path(), &inst.id).exists());
        }
        let dir = TempDir::new("g12-launch-modified").unwrap();
        let store = SqliteStore::open(dir.path(), 1).unwrap();
        let inst = instance(dir.path());
        prepare_test(&store, dir.path(), &inst).await.unwrap();
        let path = Path::new(&inst.working_directory).join("CLAUDE.md");
        fs::write(&path, b"modified by user").unwrap();
        drop(store);
        let store = SqliteStore::open(dir.path(), 2).unwrap();
        assert!(
            prepare_test(&store, dir.path(), &inst)
                .await
                .unwrap_err()
                .contains("CLAUDE.md")
        );
        assert_eq!(fs::read(path).unwrap(), b"modified by user");
    }

    #[tokio::test]
    async fn ownership_survives_reopen_and_does_not_allow_a_second_instance_to_claim_the_workspace()
    {
        let dir = TempDir::new("g12-launch-owner").unwrap();
        let store = SqliteStore::open(dir.path(), 1).unwrap();
        let mut inst = instance(dir.path());
        prepare_test(&store, dir.path(), &inst).await.unwrap();
        drop(store);
        let store = SqliteStore::open(dir.path(), 2).unwrap();
        prepare_test(&store, dir.path(), &inst).await.unwrap();
        inst.id = "g12-other".into();
        assert!(
            prepare_test(&store, dir.path(), &inst)
                .await
                .unwrap_err()
                .contains(".mcp.json")
        );
        assert!(!settings_path(dir.path(), &inst.id).exists());
    }

    #[tokio::test]
    async fn inbox_launch_is_unchanged_and_does_not_create_any_configuration() {
        let dir = TempDir::new("g12-launch-inbox").unwrap();
        let store = SqliteStore::open(dir.path(), 1).unwrap();
        let mut inst = instance(dir.path());
        inst.delivery = "inbox".into();
        prepare_test(&store, dir.path(), &inst).await.unwrap();
        assert!(!dir.path().join("claude").exists());
        assert!(
            !Path::new(&inst.working_directory)
                .join("CLAUDE.md")
                .exists()
        );
        let launched = crate::supervisor::launch(dir.path(), &inst, false).unwrap();
        assert_eq!(launched.args[..2], inst.args);
        assert_eq!(
            launched.args[2..],
            ["--session-id", inst.session_id.as_ref().unwrap()]
        );
    }

    #[tokio::test]
    async fn symlinks_hardlinks_and_symlink_settings_directories_are_refused() {
        for mode in ["symlink", "hardlink", "directory"] {
            let dir = TempDir::new("g12-launch-links").unwrap();
            let store = SqliteStore::open(dir.path(), 1).unwrap();
            let inst = instance(dir.path());
            let outside = dir.path().join("user-file");
            fs::write(&outside, b"leave alone").unwrap();
            let target = Path::new(&inst.working_directory).join("CLAUDE.md");
            match mode {
                "symlink" => symlink(&outside, target).unwrap(),
                "hardlink" => fs::hard_link(&outside, target).unwrap(),
                _ => symlink(&inst.working_directory, dir.path().join("claude")).unwrap(),
            }
            assert!(
                prepare_test(&store, dir.path(), &inst).await.is_err(),
                "{mode}"
            );
            assert_eq!(fs::read(outside).unwrap(), b"leave alone");
        }
    }

    #[tokio::test]
    async fn a_failed_ownership_commit_never_infers_ownership_from_matching_bytes() {
        let dir = TempDir::new("g12-launch-crash").unwrap();
        let store = SqliteStore::open(dir.path(), 1).unwrap();
        let inst = instance(dir.path());
        store.call(|conn| { conn.execute_batch("CREATE TEMP TRIGGER fail_owner BEFORE INSERT ON claude_owned_files BEGIN SELECT RAISE(ABORT,'injected ownership failure'); END;")?; Ok(()) }).await.unwrap();
        assert!(prepare_test(&store, dir.path(), &inst).await.is_err());
        let file = settings_path(dir.path(), &inst.id);
        let bytes = fs::read(&file).unwrap();
        drop(store);
        let store = SqliteStore::open(dir.path(), 2).unwrap();
        assert!(
            prepare_test(&store, dir.path(), &inst)
                .await
                .unwrap_err()
                .contains("foreign")
        );
        assert_eq!(fs::read(file).unwrap(), bytes);
    }

    #[test]
    fn reserved_arguments_are_refused_and_hook_executable_is_shell_quoted() {
        let dir = TempDir::new("g12-launch-args").unwrap();
        let mut inst = instance(dir.path());
        for arg in [
            "--settings=/user",
            "--permission-mode",
            "--resume",
            "--print",
            "-p",
        ] {
            inst.args = vec![arg.into()];
            assert!(args(dir.path(), &inst).unwrap_err().contains(arg));
        }
        assert_eq!(quote("/a 'b/agend"), "'/a '\\''b/agend'");
    }
}
