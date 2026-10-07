//! Observe the live needs-you view, persist identities, and deliver off-engine.
use super::{
    config::Token,
    delivery::TelegramNotifier,
    http::{Api, Method},
};
use crate::{fleet::Fleet, store::SqliteStore};
use agend_core::{
    config::TelegramConfig,
    protocol::{
        ask::{AskEntry, AskReply},
        client::AttentionRequiredData,
    },
    telegram::{TelegramDestination, TelegramNotice, TelegramStore},
    traits::{Notification, NotificationSeverity, Store},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
    time::Duration,
};
use tokio::sync::watch;

pub struct Worker {
    stop: watch::Sender<bool>,
    task: tokio::task::JoinHandle<()>,
    inbound: Option<tokio::task::JoinHandle<()>>,
}
impl Worker {
    /// Finish the current bounded HTTP call before releasing the DB for restart.
    pub fn request_stop(&self) {
        let _ = self.stop.send(true);
    }
    pub async fn stop(self) {
        self.request_stop();
        let _ = self.task.await;
        if let Some(inbound) = self.inbound {
            let _ = inbound.await;
        }
    }
}

pub fn start(config: TelegramConfig, token: Token, ctx: Arc<crate::handlers::Context>) -> Worker {
    run(
        config,
        Arc::new(Api::new(token)),
        ctx.store.clone(),
        ctx.fleet.clone(),
        Some(ctx),
    )
}

#[cfg(test)]
pub(super) fn start_with_api(
    config: TelegramConfig,
    api: Arc<Api>,
    store: Arc<SqliteStore>,
    fleet: Arc<Fleet>,
) -> Worker {
    run(config, api, store, fleet, None)
}
#[cfg(test)]
pub(crate) fn start_with_context(
    config: TelegramConfig,
    api: Arc<Api>,
    ctx: Arc<crate::handlers::Context>,
) -> Worker {
    run(config, api, ctx.store.clone(), ctx.fleet.clone(), Some(ctx))
}
fn run(
    config: TelegramConfig,
    api: Arc<Api>,
    store: Arc<SqliteStore>,
    fleet: Arc<Fleet>,
    ctx: Option<Arc<crate::handlers::Context>>,
) -> Worker {
    let (stop, mut stopped) = watch::channel(false);
    let (identity, ready) = tokio::sync::oneshot::channel();
    let inbound = ctx.map(|ctx| {
        let api = api.clone();
        let config = config.clone();
        let mut stopped = stopped.clone();
        tokio::spawn(async move {
            let destination = tokio::select! {
                _ = stopped.changed() => return,
                ready = ready => match ready { Ok(destination) => destination, Err(_) => return },
            };
            let mut reported = false;
            loop {
                if *stopped.borrow() {
                    return;
                }
                match super::poll::once(&ctx, &config, api.clone(), &destination, &stopped).await {
                    Err(error) if !reported => {
                        crate::log::line(&format!("Telegram inbound: {error}"));
                        reported = true;
                    }
                    Ok(()) => reported = false,
                    _ => {}
                }
                tokio::select! {
                    _ = stopped.changed() => return,
                    _ = tokio::time::sleep(Duration::from_secs(1)) => {},
                }
            }
        })
    });
    let task = tokio::spawn(async move {
        let mut reported = false;
        let bot_id = loop {
            if *stopped.borrow() {
                return;
            }
            let transport = api.clone();
            let result = tokio::task::spawn_blocking(move || {
                transport.call(Method::GetMe, &serde_json::json!({}))
            })
            .await;
            if let Ok(Ok(me)) = result
                && me["is_bot"].as_bool() == Some(true)
                && let Some(id) = me["id"].as_u64().filter(|id| *id > 0 && *id < (1 << 52))
            {
                break id;
            }
            if !reported {
                crate::log::line("Telegram identity unavailable; notifications remain pending");
                reported = true;
            }
            tokio::select! {
                _ = stopped.changed() => return,
                _ = tokio::time::sleep(Duration::from_secs(5)) => {},
            }
        };
        let destination = TelegramDestination {
            bot_id,
            chat_id: config.chat_id,
            topic_id: config.needs_you_topic,
        };
        let _ = identity.send(destination.clone());
        let mut notifiers = BTreeMap::new();
        for topic in std::iter::once(config.needs_you_topic)
            .chain(config.team_topics.values().copied().map(Some))
        {
            let mut routed = destination.clone();
            routed.topic_id = topic;
            notifiers
                .entry(topic)
                .or_insert_with(|| TelegramNotifier::new(api.clone(), store.clone(), routed));
        }
        let mut failures = BTreeSet::new();
        loop {
            if *stopped.borrow() {
                return;
            }
            let notices = match collect_notices(&store, &fleet, &config).await {
                Ok(n) => n,
                Err(_) => {
                    tokio::select! { _ = stopped.changed() => return, _ = tokio::time::sleep(Duration::from_secs(1)) => {} }
                    continue;
                }
            };
            match store
                .observe_telegram(&notices, &destination, crate::log::now_unix_ms())
                .await
            {
                Ok(rows) => {
                    for row in rows {
                        if *stopped.borrow() {
                            return;
                        }
                        if row.complete() {
                            continue;
                        }
                        // Reconcile again after any previous HTTP await. Only a
                        // still-current delivery may start its next part.
                        let current = match collect_notices(&store, &fleet, &config).await {
                            Ok(n) => n,
                            Err(_) => break,
                        };
                        let current_rows = match store
                            .observe_telegram(&current, &destination, crate::log::now_unix_ms())
                            .await
                        {
                            Ok(rows) => rows,
                            Err(_) => {
                                crate::log::line(
                                    "Telegram outbox unavailable; no notification sent",
                                );
                                break;
                            }
                        };
                        let Some((_, notice)) = current_rows
                            .iter()
                            .zip(&current)
                            .find(|(n, _)| n.id == row.id)
                        else {
                            continue;
                        };
                        let topic = notice.topic_id.or(destination.topic_id);
                        let Some(notifier) = notifiers.get(&topic) else {
                            continue;
                        };
                        if let Err(error) = notifier.resume_one(&row.id, &stopped).await
                            && failures.insert(row.id.clone())
                        {
                            crate::log::line(&format!("Telegram delivery {}: {error}", row.id));
                        }
                    }
                }
                Err(_) => {
                    if failures.insert("store".into()) {
                        crate::log::line("Telegram outbox unavailable; no notification sent");
                    }
                }
            }
            // Result notifications are outside attention reconciliation. A crash
            // after enqueue but before claim must not strand a never-sent row.
            for (topic, notifier) in &notifiers {
                if *stopped.borrow() {
                    return;
                }
                let mut routed = destination.clone();
                routed.topic_id = *topic;
                match store.pending_telegram(&routed, 32).await {
                    Ok(rows) => {
                        for row in rows {
                            if *stopped.borrow() {
                                return;
                            }
                            if let Err(error) = notifier.resume_one(&row.id, &stopped).await
                                && failures.insert(row.id.clone())
                            {
                                crate::log::line(&format!("Telegram delivery {}: {error}", row.id));
                            }
                        }
                    }
                    Err(_) => {
                        if failures.insert("auxiliary-store".into()) {
                            crate::log::line("Telegram auxiliary outbox unavailable; nothing sent");
                        }
                    }
                }
            }
            tokio::select! {
                _ = stopped.changed() => return,
                _ = tokio::time::sleep(Duration::from_secs(1)) => {},
            }
        }
    });
    Worker {
        stop,
        task,
        inbound,
    }
}

async fn collect_notices(
    store: &SqliteStore,
    fleet: &Fleet,
    config: &TelegramConfig,
) -> Result<Vec<TelegramNotice>, String> {
    let mut notices = Vec::new();
    let view = fleet.view();
    for item in &view.attention {
        // Do not report a broken notification channel through itself.
        if item
            .attention_id
            .as_deref()
            .is_some_and(|id| id.starts_with(crate::handlers::telegram_attention::PREFIX))
        {
            continue;
        }
        if let Some(mut notice) = notice(item) {
            if let Some(task) = &item.task_id {
                let version = store
                    .load_task(task)
                    .await
                    .map_err(|e| e.to_string())?
                    .map(|t| t.version);
                let revision = store
                    .progress(task)
                    .await
                    .map_err(|e| e.to_string())?
                    .map(|p| p.attention_revision);
                notice.task_version = version.zip(revision);
            }
            notices.push(notice);
        }
    }
    for (team, topic) in &config.team_topics {
        let mut tasks: Vec<_> = view.tasks.iter().filter(|t| &t.team_id == team).collect();
        tasks.sort_by(|a, b| a.task_id.cmp(&b.task_id));
        let body = if tasks.is_empty() {
            "No tasks.".to_owned()
        } else {
            tasks
                .into_iter()
                .map(|task| {
                    format!(
                        "{}: {}\nStatus: {}\nStage: {}",
                        task.task_id,
                        task.title,
                        task.status,
                        task.current_stage.as_deref().unwrap_or("—")
                    )
                })
                .collect::<Vec<_>>()
                .join("\n\n")
        };
        notices.push(TelegramNotice {
            key: format!("team-summary:{team}"),
            topic_id: Some(*topic),
            attention: None,
            task_version: None,
            notification: Notification {
                severity: NotificationSeverity::Info,
                title: format!("Team {team}"),
                body,
                task_id: None,
            },
        });
    }
    Ok(notices)
}

/// No boot-local wait timestamp: restarting does not change notification content.
pub(super) fn notice(item: &AttentionRequiredData) -> Option<TelegramNotice> {
    let key = item.attention_id.clone()?;
    let mut body = item.reason.clone();
    if let Some(recap) = &item.recap {
        body.push_str(&format!(
            "\n\nGoal: {}\nDecisions:\n{}\nAsking: {}\nNext: {}",
            recap.goal,
            recap.decisions.join("\n"),
            recap.asking,
            recap.next
        ));
    }
    if let Some(ask) = &item.ask {
        for entry in &ask.entries {
            match entry {
                AskEntry::Question {
                    from,
                    text,
                    options,
                }
                | AskEntry::FollowUp {
                    from,
                    text,
                    options,
                } => {
                    body.push_str(&format!("\n\n{from}: {text}"));
                    for option in options {
                        body.push_str(&format!("\n- {option}"));
                    }
                }
                AskEntry::Answer { from, reply, .. } => {
                    if let AskReply::Choice { option: text } | AskReply::Text { text } = reply {
                        body.push_str(&format!("\n\n{from}: {text}"));
                    }
                }
                AskEntry::Resolution { from, summary } => {
                    body.push_str(&format!("\n\n{from}: {summary}"))
                }
                AskEntry::Unknown => {}
            }
        }
    }
    if let Some(ignored) = &item.if_ignored {
        body.push_str(&format!("\n\nIf ignored: {ignored}"));
    }
    if let Some(unblocks) = item.unblocks {
        body.push_str(&format!("\nUnblocks: {unblocks}"));
    }
    if !item.actions.is_empty() {
        let actions: Vec<_> = item.actions.iter().map(|a| a.as_str()).collect();
        body.push_str(&format!("\nActions: {}", actions.join(", ")));
    }
    let mut attention = item.clone();
    if !attention
        .attention_id
        .as_deref()
        .is_some_and(|id| id.starts_with("instance-failed:"))
    {
        attention.waiting_since_unix_ms = None;
    }
    Some(TelegramNotice {
        topic_id: None,
        task_version: None,
        key,
        attention: Some(attention),
        notification: Notification {
            severity: NotificationSeverity::Attention,
            title: "Needs you".into(),
            body,
            task_id: item.task_id.clone(),
        },
    })
}
