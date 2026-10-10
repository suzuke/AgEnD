//! The binding is recorded under the same lock as the native spawn. Reading
//! it never infers identity from a later requested command or changes state.
use super::*;
use agend_core::protocol::holder::{BoundSpawnData, LaunchBindingData, V1_3};

fn supported(state: &State) -> bool {
    state.conn.as_ref().is_some_and(|conn| conn.version >= V1_3)
}

pub(super) fn spawn(
    holder: &Arc<Holder>,
    state: &mut State,
    data: BoundSpawnData,
) -> HolderResponse {
    if !supported(state) {
        return error(
            "unsupported_version",
            "launch binding requires holder protocol 1.3",
        );
    }
    if data.spawn.instance_id != holder.instance_id {
        return error(
            "instance_mismatch",
            "launch binding belongs to another instance",
        );
    }
    if !agend_core::protocol::client::is_uuid_v4(&data.binding) {
        return error("invalid_launch_binding", "launch binding must be a UUID v4");
    }
    let response = spawn_agent(holder, state, &data.spawn);
    if matches!(response, HolderResponse::Spawned { .. }) {
        state.launch_binding = Some(data.binding);
    }
    response
}

pub(super) fn read(holder: &Arc<Holder>, state: &State, id: &str) -> HolderResponse {
    if !supported(state) {
        return error(
            "unsupported_version",
            "launch binding requires holder protocol 1.3",
        );
    }
    if id != holder.instance_id {
        return error(
            "instance_mismatch",
            "launch binding belongs to another instance",
        );
    }
    HolderResponse::LaunchBinding {
        data: LaunchBindingData {
            instance_id: holder.instance_id.clone(),
            binding: state.launch_binding.clone(),
            process_id: state.agent.as_ref().map(|agent| agent.pid),
        },
    }
}
