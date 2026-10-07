//! Session-bound operations. HTTP acceptance and history confirmation are
//! separate: a successful prompt POST is never itself a delivery receipt.
use super::{history, http::Http};
use serde_json::{Value, json};

pub struct Session {
    http: Http,
    id: String,
}

impl Session {
    /// Attach only to the requested session. Missing context is an error, not
    /// an invitation to create a replacement conversation.
    pub fn resume(http: Http, id: &str) -> Result<Self, String> {
        if !history::valid_id(id, "ses") {
            return Err("invalid OpenCode session id".into());
        }
        let session = Self {
            http,
            id: id.into(),
        };
        session.verify()?;
        Ok(session)
    }

    /// The caller must persist this id before publishing a holder handoff or
    /// allowing any message submission. This call is never retried here.
    pub fn create(http: Http) -> Result<Self, String> {
        let created = http
            .post("/session", &json!({}))
            .map_err(|e| e.to_string())?;
        let id = created["id"]
            .as_str()
            .ok_or("OpenCode created no session id")?;
        Self::resume(http, id)
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    fn path(&self, suffix: &str) -> String {
        format!("/session/{}{suffix}", self.id)
    }

    fn verify(&self) -> Result<(), String> {
        let info = self.http.get(&self.path("")).map_err(|e| e.to_string())?;
        if info["id"] != self.id {
            return Err("OpenCode returned a different session".into());
        }
        Ok(())
    }

    pub fn history(&self) -> Result<Value, String> {
        self.verify()?;
        let value = self
            .http
            .get(&self.path("/message"))
            .map_err(|e| e.to_string())?;
        history::users(&self.id, &value)?;
        Ok(value)
    }

    /// An absent entry means idle only after this session has been found.
    pub fn busy(&self) -> Result<bool, String> {
        self.verify()?;
        let status = self
            .http
            .get("/session/status")
            .map_err(|e| e.to_string())?;
        let entries = status
            .as_object()
            .ok_or("invalid OpenCode session status")?;
        match entries.get(&self.id) {
            None => Ok(false),
            Some(value) => match value["type"].as_str() {
                Some("idle") => Ok(false),
                Some("busy" | "retry") => Ok(true),
                _ => Err("unknown OpenCode session status".into()),
            },
        }
    }

    /// Call only after the durable attempt is committed. In particular, do not
    /// call again after a timeout merely because history has not caught up.
    pub fn submit(
        &self,
        agend_id: &str,
        text: &str,
        model: Option<(&str, &str)>,
    ) -> Result<(), String> {
        if agend_id.is_empty() {
            return Err("empty AgEnD message id".into());
        }
        let mut body = json!({"messageID":history::message_id(agend_id),
            "parts":[{"type":"text","text":text}]});
        if let Some((provider, model)) = model {
            if provider.is_empty() || model.is_empty() {
                return Err("empty OpenCode model".into());
            }
            body["model"] = json!({"providerID":provider,"modelID":model});
        }
        self.http
            .post(&self.path("/prompt_async"), &body)
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn permissions(&self) -> Result<Vec<super::permission::Permission>, String> {
        self.verify()?;
        super::permission::parse(
            &self.id,
            self.http.get("/permission").map_err(|e| e.to_string())?,
        )
    }

    /// The caller must persist an operator decision and single-attempt claim
    /// before invoking this method. Never automatically repeat a lost reply.
    pub fn reply_permission(
        &self,
        expected: &super::permission::Permission,
        allow_once: bool,
    ) -> Result<(), String> {
        if expected.session != self.id || !self.permissions()?.iter().any(|p| p == expected) {
            return Err("OpenCode permission is stale or belongs to another session".into());
        }
        let result = self
            .http
            .post(
                &format!("/permission/{}/reply", expected.id),
                &json!({"reply":if allow_once {"once"} else {"reject"}}),
            )
            .map_err(|e| e.to_string())?;
        if result != true {
            return Err("OpenCode did not acknowledge permission reply".into());
        }
        Ok(())
    }

    pub fn abort(&self) -> Result<(), String> {
        let response = self
            .http
            .post(&self.path("/abort"), &json!({}))
            .map_err(|e| e.to_string())?;
        if response != true {
            return Err("OpenCode did not acknowledge abort".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agend_testkit::fake_agent::opencode::Server;
    use std::time::Duration;

    #[test]
    fn native_permissions_are_session_bound_and_stale_or_changed_requests_are_refused() {
        let server = Server::start(0, Duration::from_millis(10), None).unwrap();
        let http = || Http::new(server.port(), "fixture", "/fixture").unwrap();
        let session = Session::create(http()).unwrap();
        let foreign = Session::create(http()).unwrap();
        session
            .submit("permission-test", "run: echo permission-test", None)
            .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        let permission = loop {
            if let Some(p) = session.permissions().unwrap().into_iter().next() {
                break p;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "native producer did not ask"
            );
            std::thread::sleep(Duration::from_millis(5));
        };
        // Native observations survive SQLite reopen. A lost HTTP reply must
        // leave the exact operator decision claimed, never grant another POST.
        let dir = agend_testkit::tempdir::TempDir::new("opencode-permission-db").unwrap();
        let store = crate::store::SqliteStore::open(dir.path(), 0).unwrap();
        agend_testkit::block_on(store.add_instance(&crate::store::Instance {
            id: "open-permission".into(),
            backend: agend_core::model::Backend::Opencode,
            program: "unused".into(),
            args: vec![],
            working_directory: dir.path().display().to_string(),
            session_id: Some(session.id().into()),
            status: crate::store::InstanceStatus::Running,
            session_started: true,
            agent_pid: None,
            legacy_no_thread: false,
            delivery: "push".into(),
        }))
        .unwrap();
        let observed = permission.clone();
        let attention =
            crate::store::opencode_permissions::identity("open-permission", &permission);
        let key = attention.clone();
        store
            .call_blocking(move |conn| {
                crate::store::opencode_permissions::observe(
                    conn,
                    "open-permission",
                    &observed.session,
                    std::slice::from_ref(&observed),
                    1,
                )?;
                assert!(crate::store::opencode_permissions::claim(conn, &key, false, 2)?.is_some());
                Ok(())
            })
            .unwrap();
        drop(store);
        let store = crate::store::SqliteStore::open(dir.path(), 3).unwrap();
        let observed = permission.clone();
        store
            .call_blocking(move |conn| {
                crate::store::opencode_permissions::observe(
                    conn,
                    "open-permission",
                    &observed.session,
                    std::slice::from_ref(&observed),
                    4,
                )?;
                assert!(
                    crate::store::opencode_permissions::claim(conn, &attention, true, 5)?.is_none()
                );
                Ok(())
            })
            .unwrap();
        assert!(foreign.permissions().unwrap().is_empty());
        assert!(foreign.reply_permission(&permission, true).is_err());
        let mut changed = permission.clone();
        changed.native["metadata"]["command"] = json!("different command");
        assert!(session.reply_permission(&changed, true).is_err());
        assert_eq!(session.permissions().unwrap(), vec![permission.clone()]);
        let duplicate = json!([permission.native.clone(), permission.native.clone()]);
        assert!(super::super::permission::parse(session.id(), duplicate).is_err());
        session.reply_permission(&permission, false).unwrap();
        assert!(session.permissions().unwrap().is_empty());
        assert!(session.reply_permission(&permission, true).is_err());
        assert!(!session.busy().unwrap());
        session
            .submit("permission-handler", "run: echo handler", None)
            .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        let permission = loop {
            if let Some(p) = session.permissions().unwrap().into_iter().next() {
                break p;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        };
        let key = crate::store::opencode_permissions::identity("open-permission", &permission);
        store
            .call_blocking(move |c| {
                crate::store::opencode_permissions::observe(
                    c,
                    "open-permission",
                    &permission.session,
                    std::slice::from_ref(&permission),
                    6,
                )
            })
            .unwrap();
        crate::handlers::opencode_attention::send_decision(&store, &key, false, &session).unwrap();
        assert!(session.permissions().unwrap().is_empty());
        assert!(
            crate::handlers::opencode_attention::send_decision(&store, &key, true, &session)
                .is_err()
        );
        assert!(
            store
                .call_blocking(|c| crate::store::opencode_permissions::pending(c))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn native_session_missing_context_is_not_idle_or_replaced() {
        let server = Server::start(0, Duration::from_secs(60), None).unwrap();
        let http = || Http::new(server.port(), "fixture", "/fixture").unwrap();
        let session = Session::create(http()).unwrap();
        assert!(!session.busy().unwrap());
        assert!(Session::resume(http(), "ses_missing").is_err());
        assert!(Session::resume(http(), "ses_x/abort").is_err());
        session
            .submit("test-request", "one native request", None)
            .unwrap();
        assert!(session.busy().unwrap());
        assert_eq!(
            session.history().unwrap()[0]["parts"][0]["text"],
            "one native request"
        );
        let resumed = Session::resume(http(), session.id()).unwrap();
        assert!(resumed.busy().unwrap());
        resumed.abort().unwrap();
        assert!(!resumed.busy().unwrap());
    }
}
