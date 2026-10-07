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
