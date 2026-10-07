//! Require server-enforced up-to-date checks before attempting a merge.
use super::client::Repository;
use agend_core::{pipeline::ports::ExecutionError, traits::Runner};
use serde_json::Value;

fn strict(value: &Value) -> bool {
    let checks = &value["required_status_checks"];
    checks["strict"].as_bool() == Some(true)
        && value["enforce_admins"]["enabled"].as_bool() == Some(true)
        && value["required_linear_history"]["enabled"].as_bool() == Some(false)
        && checks["contexts"].as_array().is_some_and(|contexts| {
            !contexts.is_empty()
                && contexts
                    .iter()
                    .all(|c| c.as_str().is_some_and(|s| !s.trim().is_empty()))
        })
}

impl<R: Runner> Repository<R>
where
    R::Error: std::fmt::Display,
{
    pub(super) async fn require_strict_base(&self) -> Result<(), ExecutionError> {
        let denied = || {
            ExecutionError::Blocked(
            "GitHub base requires readable classic branch protection, strict nonempty required status checks and enforced administrators, with merge commits allowed; configure protection before retrying".into()
        )
        };
        // Production uses main. Refuse ambiguous path components rather than
        // querying a different branch's policy.
        if self.base.is_empty()
            || self.base == "."
            || self.base == ".."
            || !self
                .base
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        {
            return Err(denied());
        }
        let endpoint = self.endpoint(&format!("branches/{}/protection", self.base))?;
        let reply = self
            .api
            .request("GET", &endpoint, &[])
            .await
            .map_err(|_| denied())?;
        if reply.status != 200 || !strict(&reply.value) {
            return Err(denied());
        }
        Ok(())
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    pub(crate) fn protected() -> Value {
        let mut value: Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/github/branch-protection.json"
        ))
        .unwrap();
        // Explicit policy mutations of the actual producer capture.
        value["required_status_checks"]["strict"] = Value::Bool(true);
        value["enforce_admins"]["enabled"] = Value::Bool(true);
        value["required_linear_history"]["enabled"] = Value::Bool(false);
        value
    }
    #[test]
    fn only_strict_nonempty_checks_enforced_for_admins_allow_a_write() {
        let recorded: Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/github/branch-protection.json"
        ))
        .unwrap();
        assert!(!strict(&recorded));
        let valid = protected();
        assert!(strict(&valid));
        let mut linear = valid.clone();
        linear["required_linear_history"]["enabled"] = Value::Bool(true);
        assert!(!strict(&linear));
        for pointer in ["/required_status_checks/strict", "/enforce_admins/enabled"] {
            for bad in [
                Value::Null,
                Value::Bool(false),
                Value::String("true".into()),
            ] {
                let mut value = valid.clone();
                *value.pointer_mut(pointer).unwrap() = bad;
                assert!(!strict(&value));
            }
        }
        for bad in [
            Value::Null,
            serde_json::json!([]),
            serde_json::json!([""]),
            serde_json::json!([null]),
        ] {
            let mut value = valid.clone();
            value["required_status_checks"]["contexts"] = bad;
            assert!(!strict(&value));
        }
    }
}
