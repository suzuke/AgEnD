//! One worker per holder generation. Attempts are committed before abort or
//! prompt IO; ambiguous outcomes are reconciled against exact REST history.
use super::{api::Session, history};
use crate::store::{SqliteStore, messages, opencode};
use agend_core::{model::DeliveryState, policy::busy::BusyLevel};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

pub struct Worker {
    pub store: Arc<SqliteStore>,
    pub instance: String,
    pub session: Session,
    pub cancelled: Arc<AtomicBool>,
    pub endpoint: Option<(u32, u16, String)>,
    pub model: Option<(String, String)>,
    pub history_before: std::cell::RefCell<Option<String>>,
    pub reconcile_after: std::cell::Cell<i64>,
}
impl Worker {
    fn live(&self) -> Result<(), String> {
        if self.cancelled.load(Ordering::SeqCst) {
            return Err("OpenCode worker cancelled".into());
        }
        let id = self.instance.clone();
        let current = self
            .store
            .call_blocking(move |conn| crate::store::instances::get(conn, &id))
            .map_err(|e| e.to_string())?;
        if !current.is_some_and(|i| {
            i.backend == agend_core::model::Backend::Opencode
                && i.delivery == "push"
                && i.session_id.as_deref() == Some(self.session.id())
                && i.status == crate::store::InstanceStatus::Running
        }) {
            return Err("OpenCode worker session is no longer current".into());
        }
        Ok(())
    }

    fn allow_current_worktree(&self) -> Result<(), String> {
        use crate::store::{opencode_permissions as permissions, opencode_worktree};
        let pending = self
            .store
            .call_blocking(|c| permissions::pending(c))
            .map_err(|e| e.to_string())?;
        for p in pending.into_iter().filter(|p| {
            !p.unknown && p.instance == self.instance && p.permission.session == self.session.id()
        }) {
            let id = p.id.clone();
            let Some(grant) = self
                .store
                .call_blocking(move |c| opencode_worktree::eligible(c, &id))
                .map_err(|e| e.to_string())?
            else {
                continue;
            };
            self.live()?;
            let Some((holder, port, version)) = &self.endpoint else {
                continue;
            };
            if version != super::PERMISSION_REPLY_VERSION {
                continue;
            }
            let check_endpoint = || -> Result<(), String> {
                if crate::runtime::files::running(self.store.home(), &self.instance)
                    .map_err(|e| e.to_string())?
                    != Some(*holder)
                {
                    return Err("OpenCode permission holder changed".into());
                }
                let layout = super::launch::Layout::new(self.store.home(), &self.instance)?;
                if layout.endpoint(*holder).map_err(|e| e.to_string())? != (*port, version.clone())
                {
                    return Err("OpenCode permission endpoint changed".into());
                }
                Ok(())
            };
            check_endpoint()?;
            self.session.reply_permission(&p.permission, true, || {
                self.live()?;
                check_endpoint()?;
                let id = p.id.clone();
                let cancelled = self.cancelled.clone();
                let claimed = self
                    .store
                    .call_blocking(move |c| {
                        if cancelled.load(Ordering::SeqCst) {
                            return Ok(false);
                        }
                        opencode_worktree::claim(c, &id, &grant, crate::log::now_unix_ms())
                    })
                    .map_err(|e| e.to_string())?;
                if claimed {
                    Ok(())
                } else {
                    Err("task worktree permission is no longer current".into())
                }
            })?;
            let id = p.id;
            self.store
                .call_blocking(move |c| permissions::resolved(c, &id))
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// REST failure does not reset attempted_at. A later tick reads history
    /// again, then processes only messages that have never been attempted.
    pub fn tick(&self) -> Result<bool, String> {
        self.live()?;
        let permissions = self.session.permissions()?;
        let (id, session) = (self.instance.clone(), self.session.id().to_owned());
        self.store
            .call_blocking(move |conn| {
                crate::store::opencode_permissions::observe(
                    conn,
                    &id,
                    &session,
                    &permissions,
                    crate::log::now_unix_ms(),
                )
            })
            .map_err(|e| e.to_string())?;
        self.allow_current_worktree()?;
        let (mut history, newest_next) = self.session.history_page(16, None)?;
        let before = self.history_before.borrow().clone();
        let next = if let Some(before) = before {
            let (older, next) = self.session.history_page(16, Some(&before))?;
            let rows = history.as_array_mut().expect("validated page");
            for row in older.as_array().expect("validated page") {
                if !rows.iter().any(|r| r["info"]["id"] == row["info"]["id"]) {
                    rows.push(row.clone());
                }
            }
            next
        } else {
            newest_next
        };
        let id = self.instance.clone();
        let after = self.reconcile_after.get();
        let rows = self
            .store
            .call_blocking(move |conn| messages::opencode_page(conn, &id, true, after))
            .map_err(|e| e.to_string())?;
        for row in &rows {
            if row.attempted_at_unix_ms.is_none()
                || matches!(row.state, DeliveryState::Confirmed | DeliveryState::Failed)
            {
                continue;
            }
            let backend = history::message_id(&row.id);
            if row.turn_id.as_deref() != Some(&opencode::reference(self.session.id(), &backend)) {
                continue;
            }
            let text =
                crate::delivery::render(&row.from_instance, row.task_id.as_deref(), &row.body);
            let lookup = if history
                .as_array()
                .expect("validated")
                .iter()
                .any(|r| r["info"]["id"] == backend)
            {
                None
            } else {
                self.session
                    .message(&backend)?
                    .map(|r| serde_json::Value::Array(vec![r]))
            };
            if history::confirmed(
                self.session.id(),
                &row.id,
                &text,
                lookup.as_ref().unwrap_or(&history),
            )? {
                let (id, session) = (row.id.clone(), self.session.id().to_owned());
                self.store
                    .call_blocking(move |conn| {
                        opencode::confirm(conn, &id, &session, &backend, crate::log::now_unix_ms())
                    })
                    .map_err(|e| e.to_string())?;
            }
        }
        self.reconcile_after.set(if rows.len() < 8 {
            0
        } else {
            rows.last().unwrap().seq
        });
        let completed = history::completed(self.session.id(), &history)?;
        let (instance, session) = (self.instance.clone(), self.session.id().to_owned());
        self.store
            .call_blocking(move |c| {
                opencode::completed(
                    c,
                    &instance,
                    &session,
                    &completed,
                    crate::log::now_unix_ms(),
                )
            })
            .map_err(|e| e.to_string())?;
        *self.history_before.borrow_mut() = next;
        let mut busy = self.session.busy()?;
        let id = self.instance.clone();
        let rows = self
            .store
            .call_blocking(move |conn| messages::opencode_page(conn, &id, false, 0))
            .map_err(|e| e.to_string())?;
        for row in rows
            .iter()
            .filter(|r| r.state == DeliveryState::Queued && r.attempted_at_unix_ms.is_none())
            .take(32)
        {
            self.live()?;
            let (id, instance, session, backend) = (
                row.id.clone(),
                self.instance.clone(),
                self.session.id().to_owned(),
                history::message_id(&row.id),
            );
            if !self
                .store
                .call_blocking(move |conn| {
                    opencode::begin(
                        conn,
                        &id,
                        &instance,
                        &session,
                        &backend,
                        crate::log::now_unix_ms(),
                    )
                })
                .map_err(|e| e.to_string())?
            {
                continue;
            }
            self.live()?;
            if busy && row.level != BusyLevel::Queue {
                self.session.abort()?;
                // A queued turn can start immediately after the aborted one.
                // Successful abort authorizes the following single POST; an
                // observable idle gap is neither required nor guaranteed.
            }
            self.live()?;
            let text =
                crate::delivery::render(&row.from_instance, row.task_id.as_deref(), &row.body);
            self.session.submit(
                &row.id,
                &text,
                self.model.as_ref().map(|(p, m)| (p.as_str(), m.as_str())),
            )?;
            let (id, session, backend) = (
                row.id.clone(),
                self.session.id().to_owned(),
                history::message_id(&row.id),
            );
            self.store
                .call_blocking(move |c| {
                    opencode::accepted(c, &id, &session, &backend, crate::log::now_unix_ms())
                })
                .map_err(|e| e.to_string())?;
            busy = true;
        }
        Ok(busy)
    }
}

#[cfg(test)]
mod tests {
    use super::super::{OpenCodeDriver, http::Http};
    use super::*;
    use crate::store::{Instance, InstanceStatus};
    use agend_core::{
        model::Backend,
        traits::{AgentMessage, Driver},
    };
    use agend_testkit::{block_on, fake_agent::opencode::Server, tempdir::TempDir};
    use std::time::Duration;

    #[test]
    fn lost_native_mutation_replies_are_not_replayed_after_store_reopen() {
        for lost_abort in [false, true] {
            let dir = TempDir::new("opencode-lost-reply").unwrap();
            let server = Server::start(0, Duration::from_secs(600), None).unwrap();
            let session =
                Session::create(Http::new(server.port(), "fixture", "/fixture").unwrap()).unwrap();
            let sid = session.id().to_owned();
            let store = Arc::new(SqliteStore::open(dir.path(), 0).unwrap());
            block_on(store.add_instance(&Instance {
                id: "open-1".into(),
                backend: Backend::Opencode,
                program: "unused".into(),
                args: vec![],
                working_directory: dir.path().display().to_string(),
                session_id: Some(sid.clone()),
                status: InstanceStatus::Running,
                session_started: true,
                agent_pid: None,
                legacy_no_thread: false,
                delivery: "push".into(),
            }))
            .unwrap();
            if lost_abort {
                session.submit("prior", "prior turn", None).unwrap();
            }
            let path = format!(
                "/session/{sid}/{}",
                if lost_abort { "abort" } else { "prompt_async" }
            );
            server.lose_next_post_reply(&path);
            block_on(store.claim_message(
                &crate::store::NewMessage {
                    id: "ambiguous".into(),
                    from_instance: "sender".into(),
                    to_instance: "open-1".into(),
                    task_id: None,
                    body: "unique ambiguous payload".into(),
                    level: BusyLevel::Interrupt,
                },
                1,
            ))
            .unwrap();
            let worker = Worker {
                store: store.clone(),
                instance: "open-1".into(),
                session,
                cancelled: Arc::new(AtomicBool::new(false)),
                endpoint: None,
                model: None,
                history_before: std::cell::RefCell::new(None),
                reconcile_after: std::cell::Cell::new(0),
            };
            assert!(worker.tick().is_err(), "the committed POST reply was lost");
            assert!(
                block_on(store.message("ambiguous"))
                    .unwrap()
                    .unwrap()
                    .attempted_at_unix_ms
                    .is_some()
            );
            drop(worker);
            drop(store);
            for _boot in 0..3 {
                let store = Arc::new(SqliteStore::open(dir.path(), 0).unwrap());
                let worker = Worker {
                    store: store.clone(),
                    instance: "open-1".into(),
                    session: Session::resume(
                        Http::new(server.port(), "fixture", "/fixture").unwrap(),
                        &sid,
                    )
                    .unwrap(),
                    cancelled: Arc::new(AtomicBool::new(false)),
                    endpoint: None,
                    model: None,
                    history_before: std::cell::RefCell::new(None),
                    reconcile_after: std::cell::Cell::new(0),
                };
                for _ in 0..3 {
                    worker.tick().unwrap();
                }
                let message = block_on(store.message("ambiguous")).unwrap().unwrap();
                assert_eq!(
                    message.state,
                    if lost_abort {
                        DeliveryState::Queued
                    } else {
                        DeliveryState::Confirmed
                    }
                );
                assert_eq!(
                    server.post_count(&path),
                    1,
                    "mutation must never be replayed"
                );
                let rows = worker.session.history().unwrap();
                let users = history::users(&sid, &rows).unwrap();
                assert_eq!(users.len(), 1);
                assert_eq!(
                    server.post_count(&format!("/session/{sid}/prompt_async")),
                    1
                );
                if lost_abort {
                    assert!(
                        !history::confirmed(
                            &sid,
                            "ambiguous",
                            "From: sender\n\nunique ambiguous payload",
                            &rows
                        )
                        .unwrap()
                    );
                    assert!(
                        !worker.session.busy().unwrap(),
                        "the original abort was applied"
                    );
                }
            }
        }
    }

    #[test]
    fn oversized_total_history_does_not_block_old_receipts_or_new_delivery() {
        let dir = TempDir::new("opencode-large-history").unwrap();
        let store = Arc::new(SqliteStore::open(dir.path(), 0).unwrap());
        let server = Server::start(0, Duration::from_secs(600), None).unwrap();
        let session =
            Session::create(Http::new(server.port(), "fixture", "/fixture").unwrap()).unwrap();
        block_on(store.add_instance(&Instance {
            id: "open-1".into(),
            backend: Backend::Opencode,
            program: "unused".into(),
            args: vec![],
            working_directory: dir.path().display().to_string(),
            session_id: Some(session.id().into()),
            status: InstanceStatus::Running,
            session_started: true,
            agent_pid: None,
            legacy_no_thread: false,
            delivery: "push".into(),
        }))
        .unwrap();
        let claim = |id: &str| {
            block_on(store.claim_message(
                &crate::store::NewMessage {
                    id: id.into(),
                    from_instance: "sender".into(),
                    to_instance: "open-1".into(),
                    task_id: None,
                    body: id.into(),
                    level: BusyLevel::Queue,
                },
                1,
            ))
            .unwrap()
        };
        claim("old");
        assert!(
            block_on(store.begin_opencode_attempt(
                "old",
                "open-1",
                session.id(),
                &history::message_id("old"),
                1
            ))
            .unwrap()
        );
        session.submit("old", "From: sender\n\nold", None).unwrap();
        session.abort().unwrap();
        let large = "x".repeat(1_200_000);
        for n in 0..18 {
            session
                .submit(&format!("foreign-{n}"), &large, None)
                .unwrap();
        }
        assert!(
            session.history().is_err(),
            "unpaged history must exceed the real transport cap"
        );
        let worker = Worker {
            store: store.clone(),
            instance: "open-1".into(),
            session,
            cancelled: Arc::new(AtomicBool::new(false)),
            endpoint: None,
            model: None,
            history_before: std::cell::RefCell::new(None),
            reconcile_after: std::cell::Cell::new(0),
        };
        claim("new");
        worker.tick().unwrap();
        assert_eq!(
            block_on(store.message("old")).unwrap().unwrap().state,
            DeliveryState::Confirmed
        );
        assert_eq!(
            block_on(store.message("new")).unwrap().unwrap().state,
            DeliveryState::Sent
        );
        for _ in 0..6 {
            worker.tick().unwrap();
        }
        assert_eq!(
            block_on(store.message("new")).unwrap().unwrap().state,
            DeliveryState::Confirmed
        );
        let events = block_on(OpenCodeDriver::new(store.clone()).events("open-1", None)).unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(
                    e.kind,
                    agend_core::traits::DriverEventKind::TurnCompleted { .. }
                ))
                .count(),
            1,
            "old aborted completion backfilled once"
        );
    }

    #[test]
    fn interrupt_after_busy_queue_submits_once_even_when_backend_starts_queued_work() {
        for level in [BusyLevel::Steer, BusyLevel::Interrupt] {
            let dir = TempDir::new("opencode-interrupt").unwrap();
            let store = Arc::new(SqliteStore::open(dir.path(), 0).unwrap());
            let server = Server::start(0, Duration::from_secs(60), None).unwrap();
            let session =
                Session::create(Http::new(server.port(), "fixture", "/fixture").unwrap()).unwrap();
            block_on(store.add_instance(&Instance {
                id: "open-1".into(),
                backend: Backend::Opencode,
                program: "unused".into(),
                args: vec![],
                working_directory: dir.path().display().to_string(),
                session_id: Some(session.id().into()),
                status: InstanceStatus::Running,
                session_started: true,
                agent_pid: None,
                legacy_no_thread: false,
                delivery: "push".into(),
            }))
            .unwrap();
            let worker = Worker {
                store: store.clone(),
                instance: "open-1".into(),
                session,
                cancelled: Arc::new(AtomicBool::new(false)),
                endpoint: None,
                model: None,
                history_before: std::cell::RefCell::new(None),
                reconcile_after: std::cell::Cell::new(0),
            };
            for (id, level) in [
                ("first", BusyLevel::Queue),
                ("queued", BusyLevel::Queue),
                ("urgent", level),
            ] {
                block_on(store.claim_message(
                    &crate::store::NewMessage {
                        id: id.into(),
                        from_instance: "sender".into(),
                        to_instance: "open-1".into(),
                        task_id: None,
                        body: id.into(),
                        level,
                    },
                    1,
                ))
                .unwrap();
                assert!(
                    worker.tick().is_ok(),
                    "accepted abort must not strand the urgent message"
                );
                assert!(worker.session.busy().unwrap());
            }
            worker.tick().unwrap();
            let native = worker.session.history().unwrap();
            assert_eq!(
                history::users(worker.session.id(), &native).unwrap().len(),
                3
            );
            assert_eq!(
                native
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|r| r["info"]["error"]["name"] == "MessageAbortedError")
                    .count(),
                1
            );
            for id in ["first", "queued", "urgent"] {
                assert_eq!(
                    block_on(store.message(id)).unwrap().unwrap().state,
                    DeliveryState::Confirmed
                );
            }
            worker.tick().unwrap();
            assert_eq!(
                history::users(worker.session.id(), &worker.session.history().unwrap())
                    .unwrap()
                    .len(),
                3
            );
        }
    }

    #[test]
    fn native_worker_confirms_once_and_recovers_an_ambiguous_committed_attempt() {
        let dir = TempDir::new("opencode-worker").unwrap();
        let store = Arc::new(SqliteStore::open(dir.path(), 0).unwrap());
        let server = Server::start(0, Duration::from_secs(60), None).unwrap();
        let session =
            Session::create(Http::new(server.port(), "fixture", "/fixture").unwrap()).unwrap();
        block_on(store.add_instance(&Instance {
            id: "open-1".into(),
            backend: Backend::Opencode,
            program: "unused".into(),
            args: vec![],
            working_directory: dir.path().display().to_string(),
            session_id: Some(session.id().into()),
            status: InstanceStatus::Running,
            session_started: true,
            agent_pid: None,
            legacy_no_thread: false,
            delivery: "push".into(),
        }))
        .unwrap();
        let worker = Worker {
            store: store.clone(),
            instance: "open-1".into(),
            session,
            cancelled: Arc::new(AtomicBool::new(false)),
            endpoint: None,
            model: None,
            history_before: std::cell::RefCell::new(None),
            reconcile_after: std::cell::Cell::new(0),
        };
        let driver = OpenCodeDriver::new(store.clone());
        // More than a reconciliation page of unknown results must neither
        // consume the write batch nor hide a later accepted message forever.
        for n in 0..140 {
            let id = format!("unknown-{n}");
            block_on(store.claim_message(
                &crate::store::NewMessage {
                    id: id.clone(),
                    from_instance: "sender".into(),
                    to_instance: "open-1".into(),
                    task_id: None,
                    body: "unknown result".into(),
                    level: BusyLevel::Queue,
                },
                1,
            ))
            .unwrap();
            assert!(
                block_on(store.begin_opencode_attempt(
                    &id,
                    "open-1",
                    worker.session.id(),
                    &history::message_id(&id),
                    1
                ))
                .unwrap()
            );
        }
        let message = AgentMessage {
            id: "request-1".into(),
            from: "sender".into(),
            task_id: None,
            body: "one delivery".into(),
        };
        assert_eq!(
            block_on(driver.deliver("open-1", &message, BusyLevel::Queue))
                .unwrap()
                .state,
            DeliveryState::Queued
        );
        worker.tick().unwrap();
        // HTTP acceptance is Sent; only later exact history confirms receipt.
        assert_eq!(
            block_on(store.message("request-1")).unwrap().unwrap().state,
            DeliveryState::Sent
        );
        for _ in 0..20 {
            worker.tick().unwrap();
        }
        assert_eq!(
            block_on(driver.deliver("open-1", &message, BusyLevel::Queue))
                .unwrap()
                .state,
            DeliveryState::Confirmed
        );
        let history = worker.session.history().unwrap();
        assert_eq!(
            super::super::history::users(worker.session.id(), &history)
                .unwrap()
                .len(),
            1
        );
        let events = block_on(driver.events("open-1", None)).unwrap();
        assert_eq!(events.len(), 1);
        assert!(
            block_on(driver.events("open-1", Some(&events[0].cursor)))
                .unwrap()
                .is_empty()
        );
        worker.session.abort().unwrap();
        worker.tick().unwrap();
        let events = block_on(driver.events("open-1", None)).unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(
                    e.kind,
                    agend_core::traits::DriverEventKind::TurnCompleted { .. }
                ))
                .count(),
            1
        );
        let cursor = events.last().unwrap().cursor.clone();
        worker.tick().unwrap();
        assert!(
            block_on(driver.events("open-1", Some(&cursor)))
                .unwrap()
                .is_empty()
        );
        store
            .call_blocking(|c| {
                c.execute("DELETE FROM driver_events", [])?;
                Ok(())
            })
            .unwrap();
        worker.tick().unwrap();
        assert!(
            block_on(driver.events("open-1", None)).unwrap().is_empty(),
            "retention must not cause REST to re-emit old completions"
        );
    }
}
