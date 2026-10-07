//! Gate 10 DB projections. Every operation runs on the sole store thread.
use super::{SqliteStore, StoreError};
use agend_core::traits::TaskProgress;
use rusqlite::{OptionalExtension, params};

pub use agend_core::runtime_records::{AskRow, BindingRow, Member, Progress, Team};

impl SqliteStore {
    pub async fn advance_message(
        &self,
        id: &str,
        next: agend_core::model::DeliveryState,
        turn_id: Option<String>,
        now: u64,
    ) -> Result<(), StoreError> {
        let id = id.to_owned();
        self.call(move |c| {
            super::messages::advance(c, &id, next, turn_id.as_deref(), now)?;
            Ok(())
        })
        .await
    }

    pub async fn teams(&self) -> Result<Vec<Team>, StoreError> {
        self.call(|c| {
            let mut s = c.prepare("SELECT id,repo,default_workflow FROM teams ORDER BY id")?;
            Ok(s.query_map([], |r| {
                Ok(Team {
                    id: r.get(0)?,
                    repo: r.get(1)?,
                    default_workflow: r.get(2)?,
                })
            })?
            .collect::<Result<_, _>>()?)
        })
        .await
    }
    pub async fn add_team(&self, team: &Team) -> Result<(), StoreError> {
        let t = team.clone();
        self.call(move |c| {
            c.execute(
                "INSERT INTO teams VALUES (?1,?2,?3)",
                params![t.id, t.repo, t.default_workflow],
            )?;
            Ok(())
        })
        .await
    }
    pub async fn members(&self) -> Result<Vec<Member>, StoreError> {
        self.call(|c| {
            let mut s = c.prepare("SELECT id,team_id,role,delivery FROM instances ORDER BY id")?;
            Ok(s.query_map([], |r| {
                Ok(Member {
                    id: r.get(0)?,
                    team: r.get(1)?,
                    role: r.get(2)?,
                    delivery: r.get(3)?,
                })
            })?
            .collect::<Result<_, _>>()?)
        })
        .await
    }
    pub async fn join_team(
        &self,
        team: &str,
        instance: &str,
        role: &str,
    ) -> Result<(), StoreError> {
        let (t, i, r) = (team.to_owned(), instance.to_owned(), role.to_owned());
        self.call(move |c| {
            let busy: bool=c.query_row("SELECT EXISTS(SELECT 1 FROM tasks WHERE assignee=?1 AND status NOT IN ('done','failed','cancelled','superseded')) OR EXISTS(SELECT 1 FROM bindings WHERE instance_id=?1)",[&i],|r|r.get(0))?;
            if busy { return Err(StoreError::Invalid(format!("{i} holds a task; cancel it before changing its team or role"))); }
            if c.execute("UPDATE instances SET team_id=?1,role=?2 WHERE id=?3",params![t,r,i])? != 1 { return Err(StoreError::Invalid("unknown instance".into())); } Ok(())
        }).await
    }
    pub async fn set_team_workflow(&self, team: &str, workflow: &str) -> Result<(), StoreError> {
        let (t, w) = (team.to_owned(), workflow.to_owned());
        self.call(move |c| {
            if c.execute(
                "UPDATE teams SET default_workflow=?1 WHERE id=?2",
                params![w, t],
            )? != 1
            {
                return Err(StoreError::Invalid("unknown team".into()));
            }
            Ok(())
        })
        .await
    }
    pub async fn set_inbox_delivery(&self, instance: &str) -> Result<(), StoreError> {
        let i = instance.to_owned();
        self.call(move |c| {
            c.execute("UPDATE instances SET delivery='inbox' WHERE id=?1", [i])?;
            Ok(())
        })
        .await
    }
    pub async fn progress(&self, task: &str) -> Result<Option<Progress>, StoreError> {
        let t = task.to_owned();
        self.call(move |c| {
            Ok(c.query_row("SELECT pipeline,stage_entered_at_unix_ms,merge_intent,block_reason,attention_reason,failure_acknowledged,attention_revision FROM tasks WHERE id=?1 AND pipeline IS NOT NULL",[t],|r| Ok(Progress { attention_revision:r.get(6)?, data:TaskProgress { pipeline:r.get(0)?,stage_entered_at_unix_ms:r.get::<_,u64>(1)?,merge_intent:r.get(2)?,block_reason:r.get(3)? },block_reason:r.get(3)?,attention_reason:r.get(4)?,acknowledged:r.get(5)? })).optional()?)
        }).await
    }
    pub async fn task_note(
        &self,
        task: &str,
        block: Option<String>,
        attention: Option<String>,
        ack: bool,
    ) -> Result<(), StoreError> {
        let t = task.to_owned();
        self.call(move |c| { c.execute("UPDATE tasks SET attention_revision=attention_revision+CASE WHEN block_reason IS NOT ?1 OR attention_reason IS NOT ?2 OR failure_acknowledged IS NOT ?3 THEN 1 ELSE 0 END,block_reason=?1,attention_reason=?2,failure_acknowledged=?3 WHERE id=?4",params![block,attention,ack,t])?; Ok(()) }).await
    }
    pub async fn bindings(&self) -> Result<Vec<BindingRow>, StoreError> {
        self.call(|c| { let mut s=c.prepare("SELECT instance_id,task_id,kind,worktree,branch,head,ticket,status FROM bindings ORDER BY instance_id")?;
            Ok(s.query_map([],|r|Ok(BindingRow { instance:r.get(0)?,task:r.get(1)?,kind:r.get(2)?,worktree:r.get(3)?,branch:r.get(4)?,head:r.get(5)?,ticket:r.get(6)?,status:r.get(7)? }))?.collect::<Result<_,_>>()?) }).await
    }
    pub async fn put_binding(&self, b: &BindingRow) -> Result<(), StoreError> {
        let b = b.clone();
        self.call(move |c| { let changed = c.execute("INSERT INTO bindings VALUES (?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(instance_id) DO UPDATE SET ticket=excluded.ticket,status=excluded.status,head=excluded.head WHERE bindings.task_id=excluded.task_id AND bindings.kind=excluded.kind AND bindings.worktree=excluded.worktree",params![b.instance,b.task,b.kind,b.worktree,b.branch,b.head,b.ticket,b.status])?; if changed != 1 { return Err(StoreError::Invalid("instance already holds another binding".into())); } Ok(()) }).await
    }
    pub async fn delete_binding(&self, instance: &str) -> Result<(), StoreError> {
        let i = instance.to_owned();
        self.call(move |c| {
            c.execute("DELETE FROM bindings WHERE instance_id=?1", [i])?;
            Ok(())
        })
        .await
    }
    pub async fn latest_workflow(
        &self,
        id: &str,
    ) -> Result<Option<agend_core::pipeline::workflow::Workflow>, StoreError> {
        let id = id.to_owned();
        self.call(move |c| {
            let text: Option<String> = c
                .query_row(
                    "SELECT toml FROM workflows WHERE id=?1 ORDER BY version DESC LIMIT 1",
                    [id],
                    |r| r.get(0),
                )
                .optional()?;
            text.map(|t| toml::from_str(&t).map_err(|e| StoreError::Invalid(e.to_string())))
                .transpose()
        })
        .await
    }
    pub async fn workflow_ids(&self) -> Result<Vec<String>, StoreError> {
        self.call(|c| {
            let mut s = c.prepare("SELECT DISTINCT id FROM workflows ORDER BY id")?;
            Ok(s.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?)
        })
        .await
    }
    pub async fn held_task(&self, instance: &str) -> Result<Option<String>, StoreError> {
        let i = instance.to_owned();
        self.call(move |c| Ok(c.query_row("SELECT id FROM tasks WHERE assignee=?1 AND status NOT IN ('done','cancelled','failed','superseded') UNION SELECT task_id FROM bindings WHERE instance_id=?1 LIMIT 1",[i],|r|r.get(0)).optional()?)).await
    }
}

impl SqliteStore {
    pub async fn asks(&self) -> Result<Vec<AskRow>, StoreError> {
        self.call(|c| {
            let mut s =
                c.prepare("SELECT instance_id,created_at_unix_ms,thread FROM asks ORDER BY id")?;
            let rows = s
                .query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, u64>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            rows.into_iter()
                .map(|(instance, created, text)| {
                    Ok(AskRow {
                        instance,
                        created,
                        thread: serde_json::from_str(&text)
                            .map_err(|e| StoreError::Invalid(e.to_string()))?,
                    })
                })
                .collect()
        })
        .await
    }
    pub async fn save_ask(&self, row: &AskRow) -> Result<(), StoreError> {
        let row = row.clone();
        self.call(move |c| {
            let tx=c.transaction()?;
            let text=serde_json::to_string(&row.thread).map_err(|e|StoreError::Invalid(e.to_string()))?;
            tx.execute("INSERT INTO asks VALUES(?1,?2,?3,?4,?5) ON CONFLICT(id) DO UPDATE SET thread=excluded.thread",params![row.thread.ask_id,row.instance,row.thread.task_id,text,row.created])?;
            let count:i64=tx.query_row("SELECT COUNT(*) FROM ask_turns WHERE ask_id=?1",[&row.thread.ask_id],|r|r.get(0))?;
            for entry in row.thread.entries.iter().skip(count as usize) {
                let turn=serde_json::to_string(entry).map_err(|e|StoreError::Invalid(e.to_string()))?;
                tx.execute("INSERT INTO ask_turns(ask_id,turn) VALUES(?1,?2)",params![row.thread.ask_id,turn])?;
            }
            tx.commit()?;Ok(())
        }).await
    }
    /// Answer turns form a durable outbox, independent of message retention.
    pub async fn pending_answers(
        &self,
    ) -> Result<Vec<(u64, String, String, Option<String>, String)>, StoreError> {
        self.call(|c| {
            let mut s = c.prepare("SELECT a.seq,'ask:' || a.ask_id || '/' || (SELECT count(*) FROM ask_turns b WHERE b.ask_id=a.ask_id AND b.seq<=a.seq),q.instance_id,q.task_id,a.turn FROM ask_turns a JOIN asks q ON q.id=a.ask_id WHERE a.delivered=0 AND json_extract(a.turn,'$.entry')='answer' ORDER BY a.seq")?;
            Ok(s.query_map([], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)))?.collect::<Result<_,_>>()?)
        }).await
    }
    pub async fn mark_answer_sent(&self, seq: u64) -> Result<(), StoreError> {
        self.call(move |c| {
            c.execute("UPDATE ask_turns SET delivered=1 WHERE seq=?1", [seq])?;
            Ok(())
        })
        .await
    }
    pub async fn reminders(&self) -> Result<Vec<(u64, String, u64)>, StoreError> {
        self.call(|c| {
            let mut s =
                c.prepare("SELECT seq,task_id,due_at_unix_ms FROM reminders ORDER BY seq")?;
            Ok(s.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect::<Result<_, _>>()?)
        })
        .await
    }
    pub async fn add_reminder(&self, task: &str, due: u64) -> Result<(), StoreError> {
        let t = task.to_owned();
        let due = super::task_row::to_i64(due, "reminder due time")?;
        self.call(move |c| {
            c.execute(
                "INSERT INTO reminders(task_id,due_at_unix_ms) VALUES(?1,?2)",
                params![t, due],
            )?;
            Ok(())
        })
        .await
    }
    pub async fn delete_reminder(&self, seq: u64) -> Result<(), StoreError> {
        self.call(move |c| {
            c.execute("DELETE FROM reminders WHERE seq=?1", [seq])?;
            Ok(())
        })
        .await
    }
    pub async fn create_pipeline_task(
        &self,
        task: &agend_core::pipeline::task::Task,
        pipeline: &str,
        now: u64,
    ) -> Result<(), StoreError> {
        let task = task.clone();
        let pipeline = pipeline.to_owned();
        self.call(move |c| {
            let tx = c.transaction()?;
            super::task_row::insert(&tx, &task)?;
            tx.execute(
                "UPDATE tasks SET pipeline=?1,stage_entered_at_unix_ms=?2 WHERE id=?3",
                params![pipeline, now, task.id],
            )?;
            tx.commit()?;
            Ok(())
        })
        .await
    }
}
