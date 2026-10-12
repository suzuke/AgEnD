//! Scoped read-only setup checks in the daemon environment.
use agend_core::setup::{Check, CheckStatus};
pub(super) async fn github(ctx: &super::Context) -> Check {
    let unknown = || Check {
        check: "authentication/github".into(),
        status: CheckStatus::Warn,
        detail: "GitHub workflow scope unavailable or changed during diagnosis".into(),
        fix: Some("rerun doctor after workflow configuration settles".into()),
    };
    let Ok(scopes) = ctx.store.github_diagnostic_scopes().await else {
        return unknown();
    };
    let Some(directory) = scopes.first() else {
        return Check {
            check: "authentication/github".into(),
            status: CheckStatus::Ok,
            detail: "not required; no configured repository uses a GitHub workflow".into(),
            fix: None,
        };
    };
    let result = match crate::forge::github::api::Api::discover(ctx.runtime.home(), std::path::Path::new(directory)) {
        Ok(api) => api.authentication_status().await,
        Err(_) => Check { check: "authentication/github".into(), status: CheckStatus::Fail,
            detail: "GitHub workflow configured but real gh is unavailable in daemon PATH".into(),
            fix: Some("install gh in the daemon PATH, login with gh auth login --hostname github.com, and restart the daemon if its environment changed".into()) },
    };
    if ctx.store.github_diagnostic_scopes().await.ok().as_ref() != Some(&scopes) {
        return unknown();
    }
    result
}
