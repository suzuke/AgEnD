//! One worker per holder generation. Attempts are committed before abort or
//! prompt IO; ambiguous outcomes are reconciled against exact REST history.
use super::{api::Session, history};
use crate::store::{SqliteStore, messages, opencode};
use agend_core::{model::DeliveryState, policy::busy::BusyLevel};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

pub struct Worker {
    pub store: Arc<SqliteStore>,
    pub instance: String,
    pub session: Session,
    pub cancelled: Arc<AtomicBool>,
    pub model: Option<(String, String)>,
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
        let history = self.session.history()?;
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
            if history::confirmed(self.session.id(), &row.id, &text, &history)? {
                let (id, session) = (row.id.clone(), self.session.id().to_owned());
                self.store
                    .call_blocking(move |conn| {
                        opencode::confirm(conn, &id, &session, &backend, crate::log::now_unix_ms())
                    })
                    .map_err(|e| e.to_string())?;
            }
        }
        self.reconcile_after.set(if rows.len() < 128 {
            0
        } else {
            rows.last().unwrap().seq
        });
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
                let deadline = Instant::now() + Duration::from_secs(5);
                while self.session.busy()? {
                    self.live()?;
                    if Instant::now() >= deadline {
                        return Err(
                            "OpenCode abort did not reach idle; attempt retained without replay"
                                .into(),
                        );
                    }
                    std::thread::sleep(Duration::from_millis(25));
                }
            }
            self.live()?;
            let text =
                crate::delivery::render(&row.from_instance, row.task_id.as_deref(), &row.body);
            self.session.submit(
                &row.id,
                &text,
                self.model.as_ref().map(|(p, m)| (p.as_str(), m.as_str())),
            )?;
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
            model: None,
            reconcile_after: std::cell::Cell::new(0),
        };
        let driver = OpenCodeDriver::new(store.clone());
        // More than a reconciliation page of unknown results must neither
        // consume the write batch nor hide a later accepted message forever.
        for n in 0..140 {
            let id = format!("unknown-{n}");
            block_on(driver.deliver(
                "open-1",
                &AgentMessage {
                    id: id.clone(),
                    from: "sender".into(),
                    task_id: None,
                    body: "unknown result".into(),
                },
                BusyLevel::Queue,
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
        // The post-acceptance/pre-reconciliation window leaves the row queued,
        // while the real producer has already accepted the native request.
        assert_eq!(
            block_on(store.message("request-1")).unwrap().unwrap().state,
            DeliveryState::Queued
        );
        worker.tick().unwrap();
        worker.tick().unwrap();
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
    }
}
