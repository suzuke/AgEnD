//! Bounded operator-invoked probe of the production forge in an owned test repo.
use agend_core::{
    github::GithubStore,
    pipeline::task::Task,
    traits::{Forge, MergeRequest, Store, Submission},
};
use agend_daemon::{
    forge::github::{GithubForge, api::Api},
    git::Git,
    store::SqliteStore,
};
use std::{path::PathBuf, sync::Arc};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 6 {
        return Err(
            "usage: github_live_probe HOME REPO submit|merge|recover|cleanup TASK BRANCH".into(),
        );
    }
    let home = PathBuf::from(&args[1]);
    let repo = PathBuf::from(&args[2]).canonicalize()?;
    let operation = &args[3];
    let task = &args[4];
    let branch = &args[5];
    if !home.is_absolute() || agend_core::model::task_id_of_branch(branch) != Some(task.as_str()) {
        return Err("explicit home and matching task branch required".into());
    }
    let store = Arc::new(SqliteStore::open(&home, 0)?);
    let forge = GithubForge {
        api: Api::discover(&home, &repo)?,
        git: Git::discover(&home)?,
        repo,
        store: store.clone(),
    };
    match operation.as_str() {
        "submit" => {
            if store.load_task(task).await?.is_none() {
                store
                    .create_task(&Task::new(
                        task,
                        "AgEnD owned live fixture",
                        "general",
                        "code",
                        1,
                    ))
                    .await?;
            }
            let result = forge.submit(&Submission {
                task_id: task.clone(), branch: branch.clone(), title: "AgEnD owned live fixture".into(), body: "Synthetic test content; this repository will be deleted after verification.".into(),
            }).await.map_err(|e| e.to_string())?;
            println!(
                "{}",
                serde_json::json!({"id":result.id,"head":result.head,"url":result.url})
            );
        }
        "merge" | "recover" => {
            let record = store
                .github_change(task)
                .await?
                .ok_or("missing owned task")?;
            if record.change.identity.branch != *branch {
                return Err("branch differs from ownership".into());
            }
            let head = record.change.pushed_head.ok_or("missing pushed head")?;
            if operation == "recover" {
                println!(
                    "{}",
                    serde_json::json!({"receipt":forge.find_merge(task, &head).await?})
                );
            } else {
                match forge
                    .merge_if_head_is(&MergeRequest {
                        branch: branch.clone(),
                        expected_head: head,
                    })
                    .await
                {
                    Ok(result) => {
                        println!("{}", serde_json::json!({"result":format!("{result:?}")}))
                    }
                    Err(error) => {
                        println!("{}", serde_json::json!({"blocked":error.to_string()}));
                        std::process::exit(2);
                    }
                }
            }
        }
        "cleanup" => {
            forge.cleanup(task, true).await?;
            println!("{}", serde_json::json!({"cleaned":task}));
        }
        _ => return Err("unknown operation".into()),
    }
    Ok(())
}
