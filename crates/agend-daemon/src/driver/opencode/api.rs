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
