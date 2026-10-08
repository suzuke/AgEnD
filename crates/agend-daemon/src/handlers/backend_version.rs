//! External disk-version reminders; acknowledgement never restarts a holder.
use super::{Context, error};
use crate::{
    fleet::Fleet,
    store::{SqliteStore, StoreError},
};
use agend_core::{protocol::client::*, setup::backend::observation::SystemVersionObservation};
// Serialize publication with acknowledgement so a captured old row cannot
// republish a reminder after the operator has acknowledged it.
static PUBLICATION: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
pub(crate) const PREFIX: &str = "backend-version:";

fn item(row: &SystemVersionObservation) -> Option<AttentionRequiredData> {
    if row.revision == 0 || row.acknowledged_revision == row.revision {
        return None;
    }
    let reason = if let Some(error) = &row.error {
        format!(
            "{} external version probe failed: {error}. Any retained version is historical.",
            row.backend
        )
    } else {
        let latest = row.latest.as_ref()?;
        format!(
            "{} external program on disk changed or recovered: {} ({}). Review compatibility and run a canary before restarting; this does not identify the running holder's loaded version.",
            row.backend, latest.version_output, latest.resolved_program
        )
    };
    Some(AttentionRequiredData {
        instance_id: Some(row.instance_id.clone()),
        task_id: None,
        ask: None,
        recap: None,
        attention_id: Some(format!(
            "{PREFIX}{}/{}/{}",
            row.instance_id, row.generation, row.revision
        )),
        reason,
        unblocks: Some(0),
        waiting_since_unix_ms: row.changed_ms.or(row.completed_ms),
        if_ignored: Some(
            "Current backend versions keep running; no automatic installation or switch".into(),
        ),
        actions: vec![AttentionAction::Acknowledge],
    })
}
pub(crate) async fn refresh(store: &SqliteStore, fleet: &Fleet) -> Result<(), StoreError> {
    let _publication = PUBLICATION.lock().await;
    let mut expected = Vec::new();
    for instance in store.instances().await? {
        if let Some(launch) = store.managed_launch(&instance.id).await?
            && launch.configured_program == instance.program
        {
            continue;
        }
        if let Some(row) = store.system_version_observation(&instance.id).await?
            && row.backend == instance.backend.as_str()
            && row.program == instance.program
            && row.working_directory == instance.working_directory
            && let Some(item) = item(&row)
        {
            expected.push(item);
        }
    }
    for old in fleet.view().attention {
        if let Some(id) = old.attention_id
            && id.starts_with(PREFIX)
            && !expected
                .iter()
                .any(|item| item.attention_id.as_ref() == Some(&id))
        {
            fleet.dismiss(&id);
        }
    }
    for item in expected {
        fleet.upsert_attention(item);
    }
    Ok(())
}
async fn acknowledge(store: &SqliteStore, id: &str) -> Result<bool, StoreError> {
    let _publication = PUBLICATION.lock().await;
    let Some(rest) = id.strip_prefix(PREFIX) else {
        return Ok(false);
    };
    let parts: Vec<_> = rest.split('/').collect();
    let [instance, generation, revision] = parts.as_slice() else {
        return Ok(false);
    };
    let Ok(revision) = revision.parse::<u64>() else {
        return Ok(false);
    };
    if id != format!("{PREFIX}{instance}/{generation}/{revision}") {
        return Ok(false);
    }
    store
        .acknowledge_system_version(instance, generation, revision)
        .await
}
pub(crate) async fn resolve(ctx: &Context, data: ResolveAttentionData) -> ClientResponse {
    if data.action != AttentionAction::Acknowledge {
        return error(
            Some(data.request_id),
            error_code::INVALID_REQUEST,
            "version reminders only support acknowledge; no version is installed or switched",
        );
    }
    match acknowledge(&ctx.store, &data.attention_id).await {
        Ok(true) => {
            ctx.fleet.resolve(&data.attention_id, data.action);
            ClientResponse::CommandResult {
                data: ClientCommandResultData {
                    request_id: data.request_id,
                    result: CommandResult::Accepted,
                },
            }
        }
        Ok(false) => error(
            Some(data.request_id),
            error_code::UNKNOWN_ATTENTION,
            "version observation changed or was already acknowledged",
        ),
        Err(e) => error(
            Some(data.request_id),
            error_code::INVALID_REQUEST,
            e.to_string(),
        ),
    }
}
