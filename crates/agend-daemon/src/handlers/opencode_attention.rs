//! OpenCode permissions use the operator-only ask channel. Decisions are
//! claimed durably after read-only validation and before POST and are never replayed after an unknown result.
use super::{Context, error};
use crate::{
    driver::opencode::{api::Session, http::Http, launch::Layout},
    store::{StoreError, opencode_permissions as permissions},
};
use agend_core::protocol::{
    ask::{AskEntry, AskReply, AskThread},
    client::*,
};
use std::collections::BTreeSet;

pub(crate) const PREFIX: &str = "opencode-permission:";
const ALLOW: &str = "Allow once";
const REJECT: &str = "Reject";

pub(crate) async fn refresh(ctx: &Context) -> Result<(), StoreError> {
    let rows = ctx.store.call(|c| permissions::pending(c)).await?;
    let mut present = BTreeSet::new();
    for row in rows {
        present.insert(row.id.clone());
        let text = format!(
            "OpenCode requests {}: {}",
            row.permission.kind,
            row.permission.patterns.join("\n")
        );
        let ask = (!row.unknown).then(|| AskThread {
            ask_id: row.id.clone(),
            task_id: None,
            entries: vec![AskEntry::Question {
                from: row.instance.clone(),
                text: text.clone(),
                options: vec![ALLOW.into(), REJECT.into()],
            }],
        });
        ctx.fleet.upsert_attention(AttentionRequiredData {
            reason: if row.unknown {
                format!(
                    "Permission reply outcome unknown: {text}; it will not be resent automatically"
                )
            } else {
                text
            },
            task_id: None,
            ask,
            recap: None,
            attention_id: Some(row.id),
            unblocks: Some(1),
            waiting_since_unix_ms: Some(row.created),
            if_ignored: Some("OpenCode waits for the permission decision".into()),
            actions: vec![],
            instance_id: Some(row.instance),
        });
    }
    for item in ctx.fleet.view().attention {
        if let Some(id) = item.attention_id
            && id.starts_with(PREFIX)
            && !present.contains(&id)
        {
            ctx.fleet.dismiss(&id);
        }
    }
    Ok(())
}

pub(crate) async fn answer(ctx: &Context, data: AnswerAskData) -> ClientResponse {
    let allow = match data.reply {
        AskReply::Choice { ref option } if option == ALLOW => true,
        AskReply::Choice { ref option } if option == REJECT => false,
        _ => {
            return error(
                Some(data.request_id),
                error_code::INVALID_REQUEST,
                "choose Allow once or Reject for this permission",
            );
        }
    };
    let store = ctx.store.clone();
    let id = data.ask_id.clone();
    let result = tokio::task::spawn_blocking(move || -> Result<(), String> {
        let lookup = id.clone();
        let pending = store
            .call_blocking(|c| permissions::pending(c))
            .map_err(|e| e.to_string())?
            .into_iter()
            .find(|p| p.id == lookup && !p.unknown)
            .ok_or("permission no longer accepts an answer")?;
        let instance_id = pending.instance.clone();
        let instance = store
            .call_blocking(move |c| crate::store::instances::get(c, &instance_id))
            .map_err(|e| e.to_string())?
            .ok_or("instance removed")?;
        let holder = crate::runtime::files::running(store.home(), &instance.id)
            .map_err(|e| e.to_string())?
            .ok_or("OpenCode holder unavailable")?;
        let layout = Layout::new(store.home(), &instance.id)?;
        let (port, version) = layout.endpoint(holder).map_err(|e| e.to_string())?;
        if version != "1.18.34" {
            return Err("unverified OpenCode version".into());
        }
        let http = Http::new(
            port,
            &layout.password().map_err(|e| e.to_string())?,
            &instance.working_directory,
        )
        .map_err(|e| e.to_string())?;
        let session = Session::resume(http, &pending.permission.session)?;
        send_decision(&store, &id, allow, &session)
    })
    .await;
    let _ = refresh(ctx).await;
    match result {
        Ok(Ok(())) => ClientResponse::CommandResult {
            data: ClientCommandResultData {
                request_id: data.request_id,
                result: CommandResult::Accepted,
            },
        },
        Ok(Err(e)) => error(Some(data.request_id), error_code::INVALID_REQUEST, e),
        Err(_) => error(
            Some(data.request_id),
            error_code::INVALID_REQUEST,
            "permission reply worker stopped; inspect the retained decision",
        ),
    }
}

/// Shared by the operator handler and the native producer contract test.
pub(crate) fn send_decision(
    store: &crate::store::SqliteStore,
    id: &str,
    allow: bool,
    session: &Session,
) -> Result<(), String> {
    let permission = store
        .call_blocking(|c| permissions::pending(c))
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|p| p.id == id && !p.unknown)
        .ok_or("permission decision already claimed or stale")?
        .permission;
    session.reply_permission(&permission, allow, || {
        let claim_id = id.to_owned();
        let Some((_, claimed)) = store
            .call_blocking(move |c| {
                permissions::claim(c, &claim_id, allow, crate::log::now_unix_ms())
            })
            .map_err(|e| e.to_string())?
        else {
            return Err("permission decision already claimed or stale".into());
        };
        if claimed != permission {
            return Err("permission snapshot changed before reply".into());
        }
        Ok(())
    })?;
    let id = id.to_owned();
    store
        .call_blocking(move |c| permissions::resolved(c, &id))
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        path::Path,
        sync::{Arc, atomic::AtomicBool},
    };

    #[tokio::test]
    async fn an_agent_cannot_answer_permissions_and_operator_free_text_never_claims_one() {
        let dir = agend_testkit::tempdir::TempDir::new("opencode-operator-boundary").unwrap();
        let store = Arc::new(crate::store::SqliteStore::open(dir.path(), 0).unwrap());
        let fleet = Arc::new(crate::fleet::Fleet::new(0));
        let codex =
            crate::driver::codex::CodexDriver::new(dir.path(), store.clone(), Arc::new(|_| {}));
        let (pipeline, worker) = crate::pipeline::start(
            dir.path(),
            Path::new("/nonexistent/agend"),
            store.clone(),
            fleet.clone(),
            codex.clone(),
        )
        .await
        .unwrap();
        let (supervisor, _) = tokio::sync::mpsc::unbounded_channel();
        let ctx = Context {
            pairing: crate::notifier::pairing_service::PairingService::new(store.clone(), true),
            pipeline,
            fleet,
            runtime: crate::runtime::HolderRuntime::new(
                dir.path(),
                Path::new("/nonexistent/agend"),
                vec![],
                Arc::new(|_| {}),
            ),
            supervisor,
            store,
            codex,
            exe: "/nonexistent/agend".into(),
            restarting: AtomicBool::new(false),
            codex_input: Default::default(),
        };
        for (caller, reply, expected) in [
            (
                Some("agent"),
                AskReply::Choice {
                    option: ALLOW.into(),
                },
                error_code::FORBIDDEN,
            ),
            (
                None,
                AskReply::Text { text: ALLOW.into() },
                error_code::INVALID_REQUEST,
            ),
        ] {
            let request = ClientRequest::AnswerAsk {
                data: AnswerAskData {
                    request_id: "operator-check".into(),
                    ask_id: format!("{PREFIX}missing"),
                    source: agend_core::protocol::ask::AnswerSource::Cli,
                    reply,
                },
            };
            let super::super::Outcome::Reply(ClientResponse::Error { data }) =
                super::super::handle(&ctx, caller, request).await
            else {
                panic!("expected refusal");
            };
            assert_eq!(data.code, expected);
        }
        assert!(
            ctx.store
                .call(|c| permissions::pending(c))
                .await
                .unwrap()
                .is_empty()
        );
        worker.abort();
        let _ = worker.await;
    }
}
