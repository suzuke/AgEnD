//! REST is the reconciliation source after every reconnect. Only the exact
//! client-selected message id, session and complete text can confirm delivery.
use serde_json::Value;
use sha2::{Digest, Sha256};

#[derive(Debug, PartialEq, Eq)]
pub struct UserMessage {
    pub id: String,
    pub text: String,
    /// AgEnD sends one literal text part. Files, synthetic parts and split text
    /// cannot stand in for that request even if their visible text is equal.
    pub literal_text: bool,
}

pub fn message_id(agend_id: &str) -> String {
    format!("msg{:x}", Sha256::digest(agend_id.as_bytes()))
}

pub fn valid_id(id: &str, prefix: &str) -> bool {
    id.starts_with(prefix)
        && id.len() > prefix.len()
        && id.len() <= 256
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

pub fn users(session: &str, history: &Value) -> Result<Vec<UserMessage>, String> {
    let rows = history
        .as_array()
        .ok_or("OpenCode history is not an array")?;
    let mut result = Vec::new();
    let mut ids = std::collections::BTreeSet::new();
    for row in rows {
        let info = &row["info"];
        let id = info["id"].as_str().ok_or("OpenCode message has no id")?;
        if !valid_id(id, "msg") || !ids.insert(id) || info["sessionID"] != session {
            return Err("OpenCode history identity mismatch".into());
        }
        let role = info["role"]
            .as_str()
            .ok_or("OpenCode message has no role")?;
        if role == "assistant" {
            continue;
        }
        if role != "user" {
            return Err("unknown OpenCode message role".into());
        }
        let parts = row["parts"]
            .as_array()
            .ok_or("OpenCode message has no parts")?;
        let mut text = Vec::new();
        for part in parts {
            if part["sessionID"] != session || part["messageID"] != id {
                return Err("OpenCode part identity mismatch".into());
            }
            if part["type"] == "text" {
                text.push(
                    part["text"]
                        .as_str()
                        .ok_or("OpenCode text part has no text")?,
                );
            }
        }
        result.push(UserMessage {
            id: id.into(),
            text: text.join("\n"),
            literal_text: parts.len() == 1
                && parts[0]["type"] == "text"
                && matches!(parts[0]["synthetic"], Value::Null | Value::Bool(false))
                && matches!(parts[0]["ignored"], Value::Null | Value::Bool(false)),
        });
    }
    Ok(result)
}

/// Absence is not permission to retry a previously attempted write. The caller
/// must retain its attempt until this exact session/id/body appears or an
/// operator explicitly abandons it.
pub fn confirmed(
    session: &str,
    agend_id: &str,
    body: &str,
    history: &Value,
) -> Result<bool, String> {
    let expected = message_id(agend_id);
    for message in users(session, history)? {
        if message.id == expected {
            if !message.literal_text || message.text != body {
                return Err("OpenCode message id has different content".into());
            }
            return Ok(true);
        }
    }
    Ok(false)
}

/// Native terminal assistant records, including work completed while daemon
/// was down. Tool-call intermediate messages are not completed turns.
pub fn completed(
    session: &str,
    history: &Value,
) -> Result<Vec<(String, Option<String>, bool)>, String> {
    users(session, history)?;
    let mut result = Vec::new();
    for row in history.as_array().expect("validated") {
        let info = &row["info"];
        if info["role"] != "assistant" || !info["time"]["completed"].is_u64() {
            continue;
        }
        let error = &info["error"];
        if error.is_null()
            && !matches!(
                info["finish"].as_str(),
                Some("stop" | "length" | "content-filter")
            )
        {
            continue;
        }
        let id = info["id"].as_str().expect("validated");
        let parts = row["parts"]
            .as_array()
            .ok_or("invalid completed OpenCode parts")?;
        let mut text = Vec::new();
        for part in parts {
            if part["sessionID"] != session || part["messageID"] != id {
                return Err("completed OpenCode part identity mismatch".into());
            }
            if part["type"] == "text" {
                text.push(
                    part["text"]
                        .as_str()
                        .ok_or("invalid completed OpenCode text")?,
                );
            }
        }
        let summary = (!text.is_empty()).then(|| text.join("\n"));
        let limited = error["name"] == "APIError" && error["data"]["statusCode"] == 429;
        result.push((id.into(), summary, limited));
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::super::http::Http;
    use super::*;
    use agend_testkit::fake_agent::opencode::Server;
    use serde_json::json;
    use std::time::Duration;

    #[test]
    fn real_11834_busy_capture_keeps_three_receipts_and_two_terminal_records() {
        let history: Value =
            serde_json::from_str(include_str!("fixtures/1.18.34-busy-history.json")).unwrap();
        let session = history[0]["info"]["sessionID"].as_str().unwrap();
        let users = users(session, &history).unwrap();
        assert_eq!(users.len(), 3);
        for (user, id) in users.iter().zip([
            "a548f813-6847-4f3c-977b-9403baf911f2",
            "346da80f-adc4-40d9-9f8e-a78ae4c4fc2d",
            "d1d3dd6a-4272-4632-aae2-ca38e971c9de",
        ]) {
            assert!(confirmed(session, id, &user.text, &history).unwrap());
        }
        let turns = completed(session, &history).unwrap();
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].1, None);
        assert_eq!(turns[1].1.as_deref(), Some("AGEND_G12B_INTERRUPT_OK"));
        assert!(turns.iter().all(|t| !t.2));
    }

    #[test]
    fn real_11834_model_capture_confirms_delivery_and_one_terminal_turn() {
        let history: Value =
            serde_json::from_str(include_str!("fixtures/1.18.34-model-history.json")).unwrap();
        let session = history[0]["info"]["sessionID"].as_str().unwrap();
        let body = history[0]["parts"][0]["text"].as_str().unwrap();
        assert!(
            confirmed(
                session,
                "9d65a4ec-04c2-492c-8f2b-5181513615c1",
                body,
                &history
            )
            .unwrap()
        );
        assert_eq!(
            completed(session, &history).unwrap(),
            vec![(
                "msg_1152eb0dc001UgXCLaHbfQRzM4".into(),
                Some("AGEND_G12B_MODEL_OK".into()),
                false
            )]
        );
        let mut unfinished = history.clone();
        unfinished[1]["info"]["time"]
            .as_object_mut()
            .unwrap()
            .remove("completed");
        assert!(completed(session, &unfinished).unwrap().is_empty());
        let mut foreign = history.clone();
        foreign[1]["parts"][1]["messageID"] = json!("msg_other");
        assert!(completed(session, &foreign).is_err());
    }

    #[test]
    fn real_11834_capture_confirms_only_the_original_id_and_literal_body() {
        let history: Value =
            serde_json::from_str(include_str!("fixtures/1.18.34-no-reply-history.json")).unwrap();
        let session = history[0]["info"]["sessionID"].as_str().unwrap();
        let body = "AgEnD identity probe\n繁中 é";
        assert!(confirmed(session, "agend-identity-probe", body, &history).unwrap());
        assert!(!confirmed(session, "another-request", body, &history).unwrap());
        assert!(confirmed("ses_foreign", "agend-identity-probe", body, &history).is_err());
        assert!(confirmed(session, "agend-identity-probe", "different", &history).is_err());
        for flag in ["synthetic", "ignored"] {
            let mut changed = history.clone();
            changed[0]["parts"][0][flag] = json!(true);
            assert!(confirmed(session, "agend-identity-probe", body, &changed).is_err());
        }
        let mut changed = history.clone();
        let mut part = changed[0]["parts"][0].clone();
        part["type"] = json!("file");
        changed[0]["parts"].as_array_mut().unwrap().push(part);
        assert!(confirmed(session, "agend-identity-probe", body, &changed).is_err());
    }

    #[test]
    fn native_history_keeps_exact_identity_and_rejects_foreign_parts() {
        let server = Server::start(0, Duration::from_secs(60), None).unwrap();
        let http = Http::new(server.port(), "fixture", "/fixture").unwrap();
        let created = http.post("/session", &json!({})).unwrap();
        let session = created["id"].as_str().unwrap();
        http.post(
            &format!("/session/{session}/prompt_async"),
            &json!({"parts":[{"type":"text","text":"繁中\nsecond line"}]}),
        )
        .unwrap();
        let history = http.get(&format!("/session/{session}/message")).unwrap();
        let seen = users(session, &history).unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].text, "繁中\nsecond line");
        for (section, field, replacement) in [
            ("info", "sessionID", "ses_foreign"),
            ("parts", "sessionID", "ses_foreign"),
            ("parts", "messageID", "msg_foreign"),
        ] {
            let mut changed = history.clone();
            if section == "parts" {
                changed[0][section][0][field] = json!(replacement);
            } else {
                changed[0][section][field] = json!(replacement);
            }
            assert!(users(session, &changed).is_err());
        }
        let mut duplicated = history.clone();
        duplicated.as_array_mut().unwrap().push(history[0].clone());
        assert!(users(session, &duplicated).is_err());
        http.post(&format!("/session/{session}/abort"), &json!({}))
            .unwrap();
    }
}
