//! Permission identity is the complete native request, not just its id.
//! Polling recovers missed SSE events; replies recheck the original snapshot.
use super::history;
use serde_json::Value;
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Permission {
    pub id: String,
    pub session: String,
    pub kind: String,
    pub patterns: Vec<String>,
    pub native: Value,
}

pub fn parse(session: &str, value: Value) -> Result<Vec<Permission>, String> {
    let rows = value.as_array().ok_or("invalid OpenCode permission list")?;
    let mut ids = BTreeSet::new();
    let mut result = Vec::new();
    for row in rows {
        let id = row["id"]
            .as_str()
            .filter(|s| history::valid_id(s, "per"))
            .ok_or("invalid OpenCode permission id")?;
        let owner = row["sessionID"]
            .as_str()
            .filter(|s| history::valid_id(s, "ses"))
            .ok_or("invalid OpenCode permission session")?;
        if !ids.insert(id) {
            return Err("duplicate OpenCode permission id".into());
        }
        if owner != session {
            continue;
        }
        let kind = row["permission"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or("invalid OpenCode permission kind")?;
        let strings = |key: &str| -> Result<Vec<String>, String> {
            row[key]
                .as_array()
                .ok_or_else(|| format!("invalid OpenCode permission {key}"))?
                .iter()
                .map(|v| {
                    v.as_str()
                        .map(str::to_owned)
                        .ok_or_else(|| format!("invalid OpenCode permission {key}"))
                })
                .collect()
        };
        let patterns = strings("patterns")?;
        strings("always")?;
        if !row["metadata"].is_object() {
            return Err("invalid OpenCode permission metadata".into());
        }
        if let Some(tool) = row.get("tool")
            && (!tool["messageID"]
                .as_str()
                .is_some_and(|s| history::valid_id(s, "msg"))
                || tool["callID"].as_str().is_none_or(|s| s.is_empty()))
        {
            return Err("invalid OpenCode permission tool identity".into());
        }
        result.push(Permission {
            id: id.into(),
            session: owner.into(),
            kind: kind.into(),
            patterns,
            native: row.clone(),
        });
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_11834_permission_captures_bind_requests_and_tool_outcomes() {
        for (pending, history, status) in [
            (
                include_str!("fixtures/1.18.34-permission-once-pending.json"),
                include_str!("fixtures/1.18.34-permission-once-history.json"),
                "completed",
            ),
            (
                include_str!("fixtures/1.18.34-permission-reject-pending.json"),
                include_str!("fixtures/1.18.34-permission-reject-history.json"),
                "error",
            ),
        ] {
            let native: Value = serde_json::from_str(pending).unwrap();
            let sid = native[0]["sessionID"].as_str().unwrap();
            let parsed = parse(sid, native.clone()).unwrap();
            assert_eq!(parsed.len(), 1);
            let p = &parsed[0];
            assert_eq!(p.kind, "bash");
            assert_eq!(p.patterns.len(), 1);
            assert_eq!(p.native, native[0]);
            assert!(parse("ses_foreign", native.clone()).unwrap().is_empty());
            let rows: Value = serde_json::from_str(history).unwrap();
            super::history::users(sid, &rows).unwrap();
            let tools: Vec<_> = rows
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|m| m["parts"].as_array().unwrap())
                .filter(|p| p["type"] == "tool")
                .collect();
            assert_eq!(tools.len(), 1);
            let tool = tools[0];
            assert_eq!(tool["messageID"], p.native["tool"]["messageID"]);
            assert_eq!(tool["callID"], p.native["tool"]["callID"]);
            assert_eq!(tool["state"]["status"], status);
            assert_eq!(tool["state"]["input"]["command"], p.patterns[0]);
            let mut duplicate = native.clone();
            duplicate.as_array_mut().unwrap().push(native[0].clone());
            assert!(parse(sid, duplicate).is_err());
            let mut malformed = native.clone();
            malformed[0]["tool"]["messageID"] = "foreign".into();
            assert!(parse(sid, malformed).is_err());
        }
    }
}
