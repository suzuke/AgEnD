//! Durable permission snapshots and single-attempt operator decisions.
use super::StoreError;
use crate::driver::opencode::permission::Permission;
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};

pub fn identity(instance: &str, permission: &Permission) -> String {
    let value = serde_json::json!([
        instance,
        permission.session,
        permission.id,
        permission.native
    ]);
    format!(
        "opencode-permission:{:x}",
        Sha256::digest(value.to_string().as_bytes())
    )
}

/// Insert only observations for the still-current session. Re-observing a
/// request never resets a decision, even after daemon restart.
pub fn observe(
    conn: &Connection,
    instance: &str,
    session: &str,
    permissions: &[Permission],
    now: u64,
) -> Result<(), StoreError> {
    let tx = conn.unchecked_transaction()?;
    let current: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM instances WHERE id=?1 AND session_id=?2 AND backend='opencode' AND delivery='push' AND status='running')", params![instance,session], |r| r.get(0))?;
    if !current {
        return Err(StoreError::Invalid(
            "OpenCode permission session changed".into(),
        ));
    }
    let mut present = Vec::new();
    for p in permissions {
        if p.session != session {
            return Err(StoreError::Invalid("foreign OpenCode permission".into()));
        }
        let id = identity(instance, p);
        tx.execute("INSERT OR IGNORE INTO opencode_permissions(id,instance_id,session_id,request_id,native,status,created_at_unix_ms) VALUES(?1,?2,?3,?4,?5,'pending',?6)", params![id,instance,session,p.id,p.native.to_string(),now])?;
        present.push(id);
    }
    let mut stmt = tx.prepare("SELECT id FROM opencode_permissions WHERE instance_id=?1 AND status IN ('pending','unknown')")?;
    let old = stmt
        .query_map([instance], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    drop(stmt);
    for id in old {
        if !present.contains(&id) {
            tx.execute(
                "UPDATE opencode_permissions SET status='stale' WHERE id=?1",
                [id],
            )?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// Claims one network attempt. The original snapshot is returned only once;
/// a missing, stale, previously attempted or changed session grants no write.
pub fn claim(
    conn: &Connection,
    id: &str,
    allow: bool,
    now: u64,
) -> Result<Option<(String, Permission)>, StoreError> {
    claim_checked(conn, id, allow, now, None)
}

pub(super) fn claim_checked(
    conn: &Connection,
    id: &str,
    allow: bool,
    now: u64,
    grant: Option<&super::opencode_worktree::Grant>,
) -> Result<Option<(String, Permission)>, StoreError> {
    let tx = conn.unchecked_transaction()?;
    if let Some(grant) = grant
        && super::opencode_worktree::eligible(&tx, id)?.as_ref() != Some(grant)
    {
        return Ok(None);
    }
    let row: Option<(String,String,String)> = tx.query_row("SELECT p.instance_id,p.session_id,p.native FROM opencode_permissions p JOIN instances i ON i.id=p.instance_id WHERE p.id=?1 AND p.status='pending' AND p.attempted_at_unix_ms IS NULL AND i.session_id=p.session_id AND i.status='running' AND i.backend='opencode' AND i.delivery='push'", [id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
    let Some((instance, session, native)) = row else {
        return Ok(None);
    };
    let native: serde_json::Value =
        serde_json::from_str(&native).map_err(|e| StoreError::Invalid(e.to_string()))?;
    let p = crate::driver::opencode::permission::parse(&session, serde_json::json!([native]))
        .map_err(StoreError::Invalid)?
        .pop()
        .ok_or_else(|| StoreError::Invalid("missing stored permission".into()))?;
    if identity(&instance, &p) != id {
        return Err(StoreError::Invalid(
            "permission snapshot identity changed".into(),
        ));
    }
    tx.execute("UPDATE opencode_permissions SET decision=?2,attempted_at_unix_ms=?3,status='unknown' WHERE id=?1", params![id,if allow {"once"} else {"reject"},now])?;
    tx.commit()?;
    Ok(Some((instance, p)))
}

pub fn resolved(conn: &Connection, id: &str) -> Result<(), StoreError> {
    conn.execute("UPDATE opencode_permissions SET status='resolved' WHERE id=?1 AND status='unknown' AND attempted_at_unix_ms IS NOT NULL", [id])?;
    Ok(())
}

#[derive(Clone)]
pub struct Pending {
    pub id: String,
    pub instance: String,
    pub permission: Permission,
    pub unknown: bool,
    pub created: u64,
}

pub fn pending(conn: &Connection) -> Result<Vec<Pending>, StoreError> {
    let mut stmt = conn.prepare("SELECT p.id,p.instance_id,p.session_id,p.native,p.status,p.created_at_unix_ms FROM opencode_permissions p JOIN instances i ON i.id=p.instance_id AND i.session_id=p.session_id WHERE p.status IN ('pending','unknown') AND i.backend='opencode' AND i.delivery='push' ORDER BY p.created_at_unix_ms,p.id")?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, u64>(5)?,
        ))
    })?;
    rows.map(|r| {
        let (id, instance, session, text, status, created) = r?;
        let native: serde_json::Value =
            serde_json::from_str(&text).map_err(|e| StoreError::Invalid(e.to_string()))?;
        let permission =
            crate::driver::opencode::permission::parse(&session, serde_json::json!([native]))
                .map_err(StoreError::Invalid)?
                .pop()
                .ok_or_else(|| StoreError::Invalid("missing permission".into()))?;
        if identity(&instance, &permission) != id {
            return Err(StoreError::Invalid(
                "permission snapshot identity changed".into(),
            ));
        }
        Ok(Pending {
            id,
            instance,
            permission,
            unknown: status == "unknown",
            created,
        })
    })
    .collect()
}
