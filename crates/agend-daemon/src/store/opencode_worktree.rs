//! Task ownership authorizes only the exact active worktree's native directory
//! request. The durable once claim is the authorization linearization point.
use super::{StoreError, opencode_permissions};
use agend_core::pipeline::{
    state::{PipelineSnapshot, PipelineState, PipelineStatus},
    workflow::{Stage, Workflow},
};
use rusqlite::{Connection, OptionalExtension};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Grant {
    task: String,
    version: u64,
    ticket: String,
    worktree: String,
}

pub(crate) fn eligible(conn: &Connection, id: &str) -> Result<Option<Grant>, StoreError> {
    let Some(p) = opencode_permissions::pending(conn)?
        .into_iter()
        .find(|p| p.id == id && !p.unknown)
    else {
        return Ok(None);
    };
    if p.permission.kind != "external_directory" || p.permission.patterns.len() != 1 {
        return Ok(None);
    }
    let row = conn.query_row(
        "SELECT b.task_id,b.ticket,b.worktree,t.version,t.pipeline,w.toml,t.workflow_id,t.workflow_version FROM bindings b
         JOIN tasks t ON t.id=b.task_id JOIN workflows w ON w.id=t.workflow_id AND w.version=t.workflow_version
         WHERE b.instance_id=?1 AND b.kind='work' AND b.status='ready'
         AND t.assignee=b.instance_id AND t.status='running' AND t.requires_repo=1",
        [&p.instance], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,
            r.get::<_,u64>(3)?,r.get::<_,String>(4)?,r.get::<_,String>(5)?,r.get::<_,String>(6)?,r.get::<_,u64>(7)?)),
    ).optional()?;
    let Some((task, ticket, worktree, version, pipeline, toml, workflow_id, workflow_version)) =
        row
    else {
        return Ok(None);
    };
    let path = std::path::Path::new(&worktree);
    // Native patterns are globs. Never interpret a task path as a broader glob,
    // and never authorize aliases, missing paths or replaced directory symlinks.
    if worktree.contains(['*', '?', '[', ']', '{', '}', '\\'])
        || !path.is_absolute()
        || !path.is_dir()
        || path.canonicalize().ok().as_deref() != Some(path)
        || p.permission.patterns[0] != format!("{worktree}/*")
    {
        return Ok(None);
    }
    let workflow: Workflow =
        toml::from_str(&toml).map_err(|e| StoreError::Invalid(e.to_string()))?;
    if workflow.id != workflow_id
        || workflow.version != workflow_version
        || !workflow.requires_repo()
    {
        return Ok(None);
    }
    let snapshot: PipelineSnapshot =
        serde_json::from_str(&pipeline).map_err(|e| StoreError::Invalid(e.to_string()))?;
    let state = PipelineState::restore(
        snapshot,
        crate::pipeline::validate(workflow).map_err(|e| StoreError::Invalid(format!("{e:?}")))?,
    )
    .map_err(|e| StoreError::Invalid(format!("{e:?}")))?;
    let Some(stage) = state.current_stage() else {
        return Ok(None);
    };
    if state.task_id() != task
        || state.status() != PipelineStatus::Running
        || !matches!(stage.stage, Stage::Work { .. })
        || ticket != format!("{task}/{}/{}", stage.id, state.attempt())
    {
        return Ok(None);
    }
    Ok(Some(Grant {
        task,
        version,
        ticket,
        worktree,
    }))
}

pub(crate) fn claim(
    conn: &Connection,
    id: &str,
    grant: &Grant,
    now: u64,
) -> Result<bool, StoreError> {
    Ok(opencode_permissions::claim_checked(conn, id, true, now, Some(grant))?.is_some())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::driver::opencode::{api::Session, http::Http};
    use agend_core::pipeline::{
        state::{PipelineEvent, step},
        task::{Task, TaskStatus},
    };
    use agend_testkit::{block_on, fake_agent::opencode::Server, tempdir::TempDir};
    use std::time::{Duration, Instant};

    #[test]
    fn native_directory_requests_require_current_ownership_and_are_claimed_once() {
        exercise(false);
    }

    #[test]
    fn worker_only_replies_for_its_pinned_endpoint_and_resolves_the_request() {
        exercise(true);
    }

    fn exercise(worker_mode: bool) {
        let dir = TempDir::new("open-worktree-permission").unwrap();
        let root = dir.path().canonicalize().unwrap();
        let server = Server::start(0, Duration::from_millis(5), None).unwrap();
        let session =
            Session::create(Http::new(server.port(), "fixture", "/fixture").unwrap()).unwrap();
        session
            .submit(
                "bound-directory",
                &format!("run: external-directory: {}", root.display()),
                None,
            )
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        let permission = loop {
            if let Some(p) = session.permissions().unwrap().into_iter().next() {
                break p;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        };
        let store = std::sync::Arc::new(super::super::SqliteStore::open(&root, 0).unwrap());
        block_on(store.add_instance(&super::super::Instance {
            id: "dev".into(),
            backend: agend_core::model::Backend::Opencode,
            program: "unused".into(),
            args: vec![],
            working_directory: "/fixture".into(),
            session_id: Some(session.id().into()),
            status: super::super::InstanceStatus::Running,
            session_started: true,
            agent_pid: None,
            legacy_no_thread: false,
            delivery: "push".into(),
        }))
        .unwrap();
        let workflow = Workflow::builtin_code();
        block_on(store.save_workflow(&workflow)).unwrap();
        let state = PipelineState::new("t-1", crate::pipeline::validate(workflow).unwrap());
        let (state, _) = step(&state, PipelineEvent::Start).unwrap();
        let mut task = Task::new("t-1", "test", "general", "code", 1).set_requires_repo(true);
        task.status = TaskStatus::Running;
        task.assignee = Some("dev".into());
        block_on(store.create_pipeline_task(
            &task,
            &serde_json::to_string(&state.snapshot()).unwrap(),
            1,
        ))
        .unwrap();
        block_on(store.put_binding(&agend_core::runtime_records::BindingRow {
            instance: "dev".into(),
            task: "t-1".into(),
            kind: "work".into(),
            worktree: root.display().to_string(),
            branch: None,
            head: None,
            ticket: "t-1/work/1".into(),
            status: "ready".into(),
        }))
        .unwrap();
        let id = opencode_permissions::identity("dev", &permission);
        let p = permission.clone();
        store
            .call_blocking(move |c| {
                opencode_permissions::observe(c, "dev", &p.session.clone(), &[p], 2)
            })
            .unwrap();
        let key = id.clone();
        let grant = store
            .call_blocking(move |c| eligible(c, &key))
            .unwrap()
            .expect("active work grant");
        for field in ["id", "version", "requires_repo"] {
            let mut changed = Workflow::builtin_code();
            match field {
                "id" => changed.id = "other".into(),
                "version" => changed.version += 1,
                _ => changed.requires.clear(),
            }
            let changed = toml::to_string(&changed).unwrap();
            let original = toml::to_string(&Workflow::builtin_code()).unwrap();
            let key = id.clone();
            store
                .call_blocking(move |c| {
                    c.execute("UPDATE workflows SET toml=?1", [changed])?;
                    assert!(eligible(c, &key)?.is_none());
                    c.execute("UPDATE workflows SET toml=?1", [original])?;
                    Ok(())
                })
                .unwrap();
        }
        for (change, restore) in [
            (
                "UPDATE tasks SET status='cancelled'",
                "UPDATE tasks SET status='running'",
            ),
            (
                "UPDATE tasks SET assignee=NULL",
                "UPDATE tasks SET assignee='dev'",
            ),
            (
                "UPDATE bindings SET ticket='t-1/work/2'",
                "UPDATE bindings SET ticket='t-1/work/1'",
            ),
            (
                "UPDATE bindings SET status='pending'",
                "UPDATE bindings SET status='ready'",
            ),
            (
                "UPDATE bindings SET kind='review'",
                "UPDATE bindings SET kind='work'",
            ),
            ("UPDATE instances SET session_id='ses_foreign'", ""),
        ] {
            let key = id.clone();
            let g = grant.clone();
            let sid = session.id().to_owned();
            store
                .call_blocking(move |c| {
                    c.execute(change, [])?;
                    assert!(eligible(c, &key)?.is_none());
                    assert!(!claim(c, &key, &g, 3)?);
                    if restore.is_empty() {
                        c.execute("UPDATE instances SET session_id=?1", [sid])?;
                    } else {
                        c.execute(restore, [])?;
                    }
                    Ok(())
                })
                .unwrap();
        }
        // A valid but changed task version invalidates an earlier decision.
        let key = id.clone();
        let g = grant.clone();
        store
            .call_blocking(move |c| {
                c.execute("UPDATE tasks SET version=version+1", [])?;
                assert!(!claim(c, &key, &g, 3)?);
                c.execute("UPDATE tasks SET version=version-1", [])?;
                Ok(())
            })
            .unwrap();
        // Adversaries derive from the producer's complete request; they are
        // not independent native captures.
        for patterns in [
            vec![],
            vec!["/*".to_owned()],
            vec![format!("{}/../*", root.display())],
            vec![format!("{}/*", root.display()), "/other/*".into()],
        ] {
            let mut native = permission.native.clone();
            native["patterns"] = serde_json::json!(patterns);
            let altered = crate::driver::opencode::permission::parse(
                session.id(),
                serde_json::json!([native]),
            )
            .unwrap()
            .pop()
            .unwrap();
            let key = opencode_permissions::identity("dev", &altered);
            store
                .call_blocking(move |c| {
                    opencode_permissions::observe(
                        c,
                        "dev",
                        &altered.session.clone(),
                        &[altered],
                        3,
                    )?;
                    assert!(eligible(c, &key)?.is_none());
                    Ok(())
                })
                .unwrap();
        }
        // Use a fresh native identity after the adversarial snapshots became stale.
        let original = permission.clone();
        let key = id.clone();
        store
            .call_blocking(move |c| {
                c.execute("DELETE FROM opencode_permissions", [])?;
                opencode_permissions::observe(c, "dev", &original.session.clone(), &[original], 3)?;
                assert!(eligible(c, &key)?.is_some());
                Ok(())
            })
            .unwrap();
        if worker_mode {
            use std::{
                fs,
                os::{fd::AsRawFd, unix::fs::PermissionsExt},
                sync::{Arc, atomic::AtomicBool},
            };
            let layout = crate::driver::opencode::launch::Layout::new(&root, "dev").unwrap();
            layout.prepare().unwrap();
            let pid = std::process::id();
            let version_path = layout.root().join("version");
            fs::write(&version_path, format!("{pid}\n1.18.34\n")).unwrap();
            fs::set_permissions(&version_path, fs::Permissions::from_mode(0o600)).unwrap();
            layout.handoff(session.id(), server.port()).unwrap();
            let lock_path = crate::runtime::files::lock_path(&root, "dev");
            fs::create_dir_all(lock_path.parent().unwrap()).unwrap();
            let lock = fs::File::create(&lock_path).unwrap();
            fs::write(&lock_path, pid.to_string()).unwrap();
            // SAFETY: the test owns this descriptor and releases it on drop.
            assert_eq!(
                unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
                0
            );
            let mut worker = crate::driver::opencode::worker::Worker {
                store: store.clone(),
                instance: "dev".into(),
                session,
                cancelled: Arc::new(AtomicBool::new(false)),
                model: None,
                endpoint: Some((pid, server.port(), "unsupported".into())),
                history_before: std::cell::RefCell::new(None),
                reconcile_after: std::cell::Cell::new(0),
            };
            worker.tick().unwrap();
            assert_eq!(worker.session.permissions().unwrap().len(), 1);
            worker.endpoint = Some((pid, server.port() + 1, "1.18.34".into()));
            assert!(worker.tick().is_err());
            assert_eq!(worker.session.permissions().unwrap().len(), 1);
            worker.endpoint = Some((pid, server.port(), "1.18.34".into()));
            worker.tick().unwrap();
            assert!(worker.session.permissions().unwrap().is_empty());
            let key = id.clone();
            store
                .call_blocking(move |c| {
                    let status: String = c.query_row(
                        "SELECT status FROM opencode_permissions WHERE id=?1",
                        [&key],
                        |r| r.get(0),
                    )?;
                    assert_eq!(status, "resolved");
                    assert!(opencode_permissions::claim(c, &key, true, 5)?.is_none());
                    Ok(())
                })
                .unwrap();
            return;
        }
        // Native reread and persisted claim precede the one permission POST.
        let key = id.clone();
        let g = grant.clone();
        session
            .reply_permission(&permission, true, || {
                assert!(store.call_blocking(move |c| claim(c, &key, &g, 4)).unwrap());
                Ok(())
            })
            .unwrap();
        assert!(session.permissions().unwrap().is_empty());
        drop(store);
        let store = super::super::SqliteStore::open(&root, 0).unwrap();
        let key = id.clone();
        let g = grant;
        store
            .call_blocking(move |c| {
                assert!(!claim(c, &key, &g, 5)?);
                assert!(opencode_permissions::claim(c, &key, true, 5)?.is_none());
                assert!(
                    opencode_permissions::pending(c)?
                        .iter()
                        .any(|p| p.id == key && p.unknown)
                );
                Ok(())
            })
            .unwrap();
    }
}
