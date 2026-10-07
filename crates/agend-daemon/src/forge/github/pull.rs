//! Validate one complete pull request before trusting its head or merge receipt.
use serde_json::Value;

#[derive(Debug, PartialEq, Eq)]
pub struct Pull {
    pub number: u64,
    pub head: String,
    pub base: String,
    pub closed: bool,
    pub merged: bool,
    pub merge_commit: Option<String>,
}

pub fn full_sha(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn string<'a>(value: &'a Value, name: &str) -> Result<&'a str, String> {
    value[name]
        .as_str()
        .ok_or_else(|| format!("GitHub pull request is missing {name}"))
}

impl Pull {
    /// A list-pulls summary is insufficient: the full GET supplies merged state.
    pub fn parse(
        value: &Value,
        repository: &str,
        number: u64,
        branch: &str,
        base: &str,
    ) -> Result<Self, String> {
        let owner = repository
            .split_once('/')
            .ok_or("invalid repository identity")?
            .0;
        if number == 0
            || value["number"].as_u64() != Some(number)
            || string(value, "html_url")?
                != format!("https://github.com/{repository}/pull/{number}")
        {
            return Err("GitHub pull request identity differs".into());
        }
        let mut repository_id = None;
        for (side, expected_ref) in [("head", branch), ("base", base)] {
            let object = &value[side];
            let repo = &object["repo"];
            let id = repo["id"]
                .as_u64()
                .filter(|id| *id > 0)
                .ok_or("missing GitHub repository id")?;
            if string(object, "ref")? != expected_ref
                || string(object, "label")? != format!("{owner}:{expected_ref}")
                || string(repo, "full_name")? != repository
                || string(repo, "html_url")? != format!("https://github.com/{repository}")
                || repository_id.is_some_and(|previous| previous != id)
                || !full_sha(string(object, "sha")?)
            {
                return Err(format!("GitHub pull request {side} identity differs"));
            }
            repository_id = Some(id);
        }
        let closed = match string(value, "state")? {
            "open" => false,
            "closed" => true,
            _ => return Err("unknown GitHub pull request state".into()),
        };
        let merged = value["merged"]
            .as_bool()
            .ok_or("missing GitHub merged state")?;
        let merge_commit = match value.get("merge_commit_sha") {
            Some(Value::Null) => None,
            Some(Value::String(sha)) if full_sha(sha) => Some(sha.clone()),
            _ => return Err("invalid GitHub merge commit".into()),
        };
        if merged && (!closed || merge_commit.is_none()) {
            return Err("incomplete GitHub merge receipt".into());
        }
        Ok(Self {
            number,
            head: string(&value["head"], "sha")?.into(),
            base: string(&value["base"], "sha")?.into(),
            closed,
            merged,
            merge_commit,
        })
    }

    pub(super) fn receipt_for(&self, approved: &str) -> Result<Option<&str>, String> {
        if !full_sha(approved) || self.head != approved {
            return Err("GitHub receipt does not match the approved head".into());
        }
        Ok(if self.merged {
            self.merge_commit.as_deref()
        } else {
            None
        })
    }

    /// A reported merge must contain the approved head as its second parent.
    pub fn verify_merge_commit<'a>(
        &'a self,
        approved: &str,
        commit: &Value,
    ) -> Result<&'a str, String> {
        let sha = self
            .receipt_for(approved)?
            .ok_or("pull request is not merged")?;
        let parents = commit["parents"]
            .as_array()
            .ok_or("missing merge parents")?;
        if string(commit, "sha")? != sha
            || parents.len() != 2
            || !full_sha(string(&parents[0], "sha")?)
            || string(&parents[1], "sha")? != approved
        {
            return Err("GitHub merge commit does not contain the approved head".into());
        }
        Ok(sha)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn recorded() -> Value {
        let raw = include_str!("../../../tests/fixtures/github/pull.http");
        serde_json::from_str(raw.split_once("\r\n\r\n").unwrap().1).unwrap()
    }
    fn parse(value: &Value) -> Result<Pull, String> {
        Pull::parse(value, "suzuke/AgEnD", 155, "feat/g12b-opencode", "v2")
    }
    #[test]
    fn actual_pull_head_is_distinct_from_a_successful_merge_receipt() {
        let value = recorded();
        let pull = parse(&value).unwrap();
        assert!(!pull.merged);
        assert_eq!(pull.receipt_for(&pull.head).unwrap(), None);
        assert!(pull.receipt_for(&pull.head[..7]).is_err());
        assert!(pull.receipt_for(&"0".repeat(40)).is_err());
    }
    #[test]
    fn foreign_refs_repositories_and_partial_heads_cannot_supply_a_receipt() {
        for (pointer, replacement) in [
            ("/number", serde_json::json!(156)),
            ("/html_url", serde_json::json!("https://other/pull/155")),
            ("/head/ref", serde_json::json!("other")),
            ("/base/ref", serde_json::json!("other")),
            ("/head/sha", serde_json::json!("1234567")),
            ("/head/repo/full_name", serde_json::json!("foreign/AgEnD")),
            ("/head/repo/id", serde_json::json!(1)),
            (
                "/head/label",
                serde_json::json!("foreign:feat/g12b-opencode"),
            ),
            ("/merged", Value::Null),
            ("/merge_commit_sha", serde_json::json!("1234567")),
        ] {
            let mut value = recorded();
            *value.pointer_mut(pointer).unwrap() = replacement;
            assert!(parse(&value).is_err(), "{pointer}");
        }
    }

    #[test]
    fn actual_merged_pull_requires_its_matching_commit_and_approved_parent() {
        let value: Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/github/merged-pull.json"
        ))
        .unwrap();
        let commit: Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/github/merge-commit.json"
        ))
        .unwrap();
        let pull = Pull::parse(
            &value,
            "suzuke/AgEnD",
            154,
            "test/g12a-smoke-contract",
            "v2",
        )
        .unwrap();
        assert_eq!(
            pull.verify_merge_commit(&pull.head, &commit).unwrap(),
            "a6cdb4c64b244ca86a23de0a2139a4eee29e5d54"
        );
        for pointer in ["/sha", "/parents/1/sha"] {
            let mut altered = commit.clone();
            *altered.pointer_mut(pointer).unwrap() = serde_json::json!("0".repeat(40));
            assert!(pull.verify_merge_commit(&pull.head, &altered).is_err());
        }
        let mut squash = commit.clone();
        squash["parents"].as_array_mut().unwrap().remove(1);
        assert!(pull.verify_merge_commit(&pull.head, &squash).is_err());
    }
}
