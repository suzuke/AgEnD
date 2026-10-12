//! Durable registry reminders. Acknowledgement never installs or switches.
use super::{Context, error};
use crate::{
    fleet::Fleet,
    store::{SqliteStore, StoreError},
};
use agend_core::{
    model::Backend, protocol::client::*, setup::backend::observation::RegistryObservation,
};
// Serialize publication with acknowledgement so a captured old row cannot
// republish a reminder after the operator has acknowledged it.
static PUBLICATION: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
pub(crate) const PREFIX: &str = "backend-registry:";

fn item(row: &RegistryObservation) -> Option<AttentionRequiredData> {
    if row.revision == 0 || row.acknowledged_revision == row.revision {
        return None;
    }
    let reason = if let Some(error) = &row.error {
        format!(
            "{} public version check failed: {error}. Any retained version is historical.",
            row.backend
        )
    } else {
        let latest = row.latest.as_ref()?;
        format!(
            "{} registry latest is {}. Review compatibility and run a canary before choosing an upgrade.",
            row.backend, latest.version
        )
    };
    Some(AttentionRequiredData {
        instance_id: None,
        task_id: None,
        ask: None,
        recap: None,
        attention_id: Some(format!("{PREFIX}{}/{}", row.backend, row.revision)),
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
    for backend in Backend::ALL {
        if let Some(row) = store.registry_observation(backend).await?
            && !already_current(store, fleet, &row).await?
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
async fn already_current(
    store: &SqliteStore,
    fleet: &Fleet,
    row: &RegistryObservation,
) -> Result<bool, StoreError> {
    if row.error.is_some() {
        return Ok(false);
    }
    let Some(latest) = &row.latest else {
        return Ok(false);
    };
    let instances: Vec<_> = fleet
        .view()
        .instances
        .into_iter()
        .filter(|i| i.backend == row.backend)
        .collect();
    if instances.is_empty() {
        return Ok(false);
    }
    for instance in instances {
        let Some(launch) = store.managed_launch(&instance.instance_id).await? else {
            return Ok(false);
        };
        if instance.program.as_deref() != Some(launch.configured_program.as_str())
            || launch.artifact.backend != row.backend
            || launch.artifact.version != latest.version
        {
            return Ok(false);
        }
    }
    Ok(true)
}

async fn acknowledge(store: &SqliteStore, id: &str) -> Result<bool, StoreError> {
    let _publication = PUBLICATION.lock().await;
    let Some((backend, revision)) = id
        .strip_prefix(PREFIX)
        .and_then(|rest| rest.split_once('/'))
    else {
        return Ok(false);
    };
    let (Some(backend), Ok(revision)) = (Backend::parse(backend), revision.parse::<u64>()) else {
        return Ok(false);
    };
    if id != format!("{PREFIX}{}/{revision}", backend.as_str()) {
        return Ok(false);
    }
    store
        .acknowledge_registry_observation(backend, revision)
        .await
}
pub(crate) async fn resolve(ctx: &Context, data: ResolveAttentionData) -> ClientResponse {
    if data.action != AttentionAction::Acknowledge {
        return error(
            Some(data.request_id),
            error_code::INVALID_REQUEST,
            "registry reminders only support acknowledge; no version is installed or switched",
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
            "registry observation changed or was already acknowledged",
        ),
        Err(e) => error(
            Some(data.request_id),
            error_code::INVALID_REQUEST,
            e.to_string(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agend_core::setup::backend::{PublishedBackend, observation::REGISTRY_INTERVAL_MS};
    use agend_testkit::{block_on, tempdir::TempDir};
    #[test]
    fn native_observation_restores_and_exact_ack_cannot_hide_a_new_revision() {
        block_on(async {
            let dir = TempDir::new("registry-notice").unwrap();
            let home = dir.path().join("home");
            let store = SqliteStore::open(&home, 100).unwrap();
            let native: serde_json::Value = serde_json::from_slice(include_bytes!(
                "../../tests/fixtures/backend_registry/codex.json"
            ))
            .unwrap();
            let release = PublishedBackend {
                backend: "codex".into(),
                package: native["name"].as_str().unwrap().into(),
                version: native["version"].as_str().unwrap().into(),
            };
            let row = store
                .begin_registry_check(Backend::Codex, 100)
                .await
                .unwrap()
                .unwrap();
            store
                .finish_registry_check(Backend::Codex, row.attempt, 101, Ok(release))
                .await
                .unwrap();
            let fleet = Fleet::new(100);
            refresh(&store, &fleet).await.unwrap();
            let first = fleet.view().attention;
            assert_eq!(first.len(), 1);
            assert_eq!(first[0].actions, vec![AttentionAction::Acknowledge]);
            let id = first[0].attention_id.as_deref().unwrap();
            drop(store);
            let store = SqliteStore::open(&home, 200).unwrap();
            let restored = Fleet::new(200);
            refresh(&store, &restored).await.unwrap();
            assert_eq!(restored.view().attention, first);
            let now = 100 + REGISTRY_INTERVAL_MS;
            let second = store
                .begin_registry_check(Backend::Codex, now)
                .await
                .unwrap()
                .unwrap();
            refresh(&store, &restored).await.unwrap();
            assert_eq!(
                restored.view().attention,
                first,
                "pending daily check must not refresh the notification timestamp"
            );
            store
                .finish_registry_check(
                    Backend::Codex,
                    second.attempt,
                    now + 1,
                    Err("unavailable".into()),
                )
                .await
                .unwrap();
            assert!(!acknowledge(&store, id).await.unwrap());
            refresh(&store, &restored).await.unwrap();
            let current = restored.view().attention;
            assert_eq!(current.len(), 1);
            assert!(current[0].reason.contains("historical"));
            assert!(
                acknowledge(&store, current[0].attention_id.as_deref().unwrap())
                    .await
                    .unwrap()
            );
            refresh(&store, &restored).await.unwrap();
            assert!(restored.view().attention.is_empty());
            drop(store);
            let store = SqliteStore::open(&home, now + 2).unwrap();
            refresh(&store, &Fleet::new(now + 2)).await.unwrap();
            assert!(
                item(
                    &store
                        .registry_observation(Backend::Codex)
                        .await
                        .unwrap()
                        .unwrap()
                )
                .is_none()
            );
        });
    }
    #[test]
    fn only_an_entirely_current_managed_fleet_suppresses_the_notice() {
        use agend_core::{
            runtime_records::{Instance, InstanceStatus},
            setup::backend::ImportedBackend,
            traits::HolderLaunch,
        };
        block_on(async {
            let dir = TempDir::new("registry-current-fleet").unwrap();
            let store = SqliteStore::open(dir.path(), 0).unwrap();
            let fleet = Fleet::new(0);
            let manifest: serde_json::Value = serde_json::from_slice(include_bytes!(
                "../../tests/fixtures/backend_registry/codex.json"
            ))
            .unwrap();
            let release = PublishedBackend {
                backend: "codex".into(),
                package: manifest["name"].as_str().unwrap().into(),
                version: manifest["version"].as_str().unwrap().into(),
            };
            let attempt = store
                .begin_registry_check(Backend::Codex, 100)
                .await
                .unwrap()
                .unwrap();
            store
                .finish_registry_check(Backend::Codex, attempt.attempt, 101, Ok(release.clone()))
                .await
                .unwrap();
            for (id, version, managed) in [
                ("current", release.version.as_str(), true),
                ("older", "0.0.1", true),
                ("external", release.version.as_str(), false),
            ] {
                let instance = Instance {
                    id: id.into(),
                    backend: Backend::Codex,
                    program: format!("/managed/{id}"),
                    args: vec![],
                    working_directory: "/workspace".into(),
                    session_id: None,
                    status: InstanceStatus::Failed,
                    session_started: false,
                    agent_pid: None,
                    legacy_no_thread: false,
                    delivery: "push".into(),
                };
                store.add_instance(&instance).await.unwrap();
                if managed {
                    store
                        .prepare_managed_launch(
                            &instance,
                            &HolderLaunch {
                                instance_id: instance.id.clone(),
                                backend: instance.backend,
                                executable: instance.program.clone(),
                                args: vec![],
                                working_directory: instance.working_directory.clone(),
                            },
                            ImportedBackend {
                                format: 1,
                                backend: "codex".into(),
                                version: version.into(),
                                sha256: "a".repeat(64),
                                bytes: 1,
                            },
                            None,
                        )
                        .await
                        .unwrap();
                }
                fleet.set_instance(
                    InstanceView {
                        program: Some(instance.program),
                        instance_id: id.into(),
                        team_id: "general".into(),
                        backend: "codex".into(),
                        state: AgentState::Idle,
                        working_directory: Some(instance.working_directory),
                    },
                    "fixture".into(),
                );
                refresh(&store, &fleet).await.unwrap();
                assert_eq!(fleet.view().attention.is_empty(), id == "current");
                if id != "current" {
                    fleet.remove_instance(id);
                }
            }
            refresh(&store, &fleet).await.unwrap();
            assert!(fleet.view().attention.is_empty());
            assert_eq!(
                store
                    .registry_observation(Backend::Codex)
                    .await
                    .unwrap()
                    .unwrap()
                    .acknowledged_revision,
                0,
                "suppression does not acknowledge a version for future fleet changes"
            );
        });
    }
}
