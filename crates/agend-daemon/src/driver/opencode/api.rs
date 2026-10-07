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

    /// A bounded native history page. The cursor is opaque and endpoint-bound.
    pub fn history_page(
        &self,
        limit: usize,
        before: Option<&str>,
    ) -> Result<(Value, Option<String>), String> {
        self.verify()?;
        let mut size = limit;
        let (rows, next) = loop {
            match self.http.page(&self.path("/message"), size, before) {
                Ok(page) => break page,
                Err(super::http::Error::TooLarge) if size > 1 => size = (size / 2).max(1),
                Err(e) => return Err(e.to_string()),
            }
        };
        history::users(&self.id, &rows)?;
        if rows.as_array().expect("validated").len() > size {
            return Err("OpenCode page exceeds requested limit".into());
        }
        Ok((rows, next))
    }

    /// Lookup an old attempt without downloading the complete session.
    pub fn message(&self, id: &str) -> Result<Option<Value>, String> {
        if !history::valid_id(id, "msg") {
            return Err("invalid OpenCode message id".into());
        }
        self.verify()?;
        let row = match self.http.get(&self.path(&format!("/message/{id}"))) {
            Ok(row) => row,
            Err(super::http::Error::Status(404)) => return Ok(None),
            Err(e) => return Err(e.to_string()),
        };
        if row["info"]["id"] != id {
            return Err("OpenCode returned a different message".into());
        }
        history::users(&self.id, &Value::Array(vec![row.clone()]))?;
        Ok(Some(row))
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

    /// Validate with read-only IO before claiming the one POST attempt.
    /// The callback must durably claim this exact snapshot or refuse the write.
    pub fn reply_permission(
        &self,
        expected: &super::permission::Permission,
        allow_once: bool,
        claim: impl FnOnce() -> Result<(), String>,
    ) -> Result<(), String> {
        if expected.session != self.id || !self.permissions()?.iter().any(|p| p == expected) {
            return Err("OpenCode permission is stale or belongs to another session".into());
        }
        claim()?;
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
        assert!(
            foreign
                .reply_permission(&permission, true, || Ok(()))
                .is_err()
        );
        let mut changed = permission.clone();
        changed.native["metadata"]["command"] = json!("different command");
        assert!(session.reply_permission(&changed, true, || Ok(())).is_err());
        assert_eq!(session.permissions().unwrap(), vec![permission.clone()]);
        let duplicate = json!([permission.native.clone(), permission.native.clone()]);
        assert!(super::super::permission::parse(session.id(), duplicate).is_err());
        session
            .reply_permission(&permission, false, || Ok(()))
            .unwrap();
        assert!(session.permissions().unwrap().is_empty());
        assert!(
            session
                .reply_permission(&permission, true, || Ok(()))
                .is_err()
        );
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
        // Read-only validation failures have not attempted the reply. The
        // operator must still be able to answer after the endpoint recovers.
        for path in [format!("/session/{}", session.id()), "/permission".into()] {
            server.fail_next_get(&path);
            assert!(
                crate::handlers::opencode_attention::send_decision(&store, &key, false, &session)
                    .is_err()
            );
            let pending = store
                .call_blocking(|c| crate::store::opencode_permissions::pending(c))
                .unwrap();
            assert_eq!(pending.len(), 1);
            assert!(
                !pending[0].unknown,
                "GET failure must not consume a POST attempt"
            );
            assert_eq!(
                server.post_count(&format!("/permission/{}/reply", pending[0].permission.id)),
                0
            );
        }
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

        session
            .submit("lost-permission-reply", "run: echo lost", None)
            .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        let permission = loop {
            if let Some(p) = session.permissions().unwrap().into_iter().next() {
                break p;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        };
        let reply_path = format!("/permission/{}/reply", permission.id);
        let key = crate::store::opencode_permissions::identity("open-permission", &permission);
        store
            .call_blocking(move |c| {
                crate::store::opencode_permissions::observe(
                    c,
                    "open-permission",
                    &permission.session,
                    std::slice::from_ref(&permission),
                    7,
                )
            })
            .unwrap();
        server.lose_next_post_reply(&reply_path);
        assert!(
            crate::handlers::opencode_attention::send_decision(&store, &key, false, &session)
                .is_err()
        );
        assert_eq!(server.post_count(&reply_path), 1);
        assert!(
            session.permissions().unwrap().is_empty(),
            "backend applied the reply"
        );
        drop(store);
        let store = crate::store::SqliteStore::open(dir.path(), 8).unwrap();
        let pending = store
            .call_blocking(|c| crate::store::opencode_permissions::pending(c))
            .unwrap();
        assert_eq!(pending.len(), 1);
        assert!(pending[0].unknown);
        assert!(
            crate::handlers::opencode_attention::send_decision(&store, &key, true, &session)
                .is_err()
        );
        assert_eq!(
            server.post_count(&reply_path),
            1,
            "lost POST response must not grant another attempt"
        );
    }

    #[test]
    fn native_finished_turns_backfill_and_reject_foreign_assistant_parts() {
        let server = Server::start(0, Duration::from_millis(10), None).unwrap();
        let session =
            Session::create(Http::new(server.port(), "fixture", "/fixture").unwrap()).unwrap();
        session.submit("complete-test", "one word", None).unwrap();
        let end = std::time::Instant::now() + Duration::from_secs(2);
        while session.busy().unwrap() {
            assert!(std::time::Instant::now() < end);
            std::thread::sleep(Duration::from_millis(5));
        }
        let mut history = session.history().unwrap();
        let finished = super::super::history::completed(session.id(), &history).unwrap();
        assert_eq!(finished.len(), 1);
        assert!(
            finished[0]
                .1
                .as_deref()
                .is_some_and(|s| s.contains("one word"))
        );
        let assistant = history
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|r| r["info"]["role"] == "assistant")
            .unwrap();
        assistant["parts"][0]["sessionID"] = json!("ses_foreign");
        assert!(super::super::history::completed(session.id(), &history).is_err());
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

#[cfg(test)]
mod pagination_capture_tests {
    use super::*;
    use std::{
        io::{BufRead, BufReader, Write},
        net::TcpListener,
    };

    #[test]
    fn captured_native_pages_and_old_message_use_only_the_original_endpoint() {
        let captured: Value =
            serde_json::from_str(include_str!("fixtures/1.18.34-pages.json")).unwrap();
        let sid = captured["session"]["id"].as_str().unwrap().to_owned();
        let fixture = captured.clone();
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let producer = std::thread::spawn(move || {
            let mut page = 0;
            for _ in 0..7 {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut request = String::new();
                reader.read_line(&mut request).unwrap();
                loop {
                    let mut h = String::new();
                    reader.read_line(&mut h).unwrap();
                    if h == "\r\n" {
                        break;
                    }
                }
                let (body, next) = if request.contains("/message?") {
                    assert!(request.contains("limit=2"));
                    let p = &fixture["pages"][page];
                    if page == 1 {
                        assert!(request.contains(p["before"].as_str().unwrap()));
                    }
                    page += 1;
                    (p["rows"].to_string(), p["next"].as_str().map(str::to_owned))
                } else if request.contains("/message/") {
                    assert!(request.contains(fixture["single"]["info"]["id"].as_str().unwrap()));
                    (fixture["single"].to_string(), None)
                } else {
                    (fixture["session"].to_string(), None)
                };
                let cursor = next
                    .map(|c| format!("X-Next-Cursor: {c}\r\n"))
                    .unwrap_or_default();
                write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\nLink: <http://127.0.0.1:1/foreign>; rel=\"next\"\r\n{cursor}\r\n{body}",body.len()).unwrap();
            }
            assert_eq!(page, 2);
        });
        let session =
            Session::resume(Http::new(port, "fixture", "/fixture").unwrap(), &sid).unwrap();
        let (first, next) = session.history_page(2, None).unwrap();
        assert_eq!(first, captured["pages"][0]["rows"]);
        let (second, end) = session.history_page(2, next.as_deref()).unwrap();
        assert_eq!(second, captured["pages"][1]["rows"]);
        assert!(end.is_none());
        let old = session
            .message(captured["single"]["info"]["id"].as_str().unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(old, captured["single"]);
        producer.join().unwrap();
    }
}
