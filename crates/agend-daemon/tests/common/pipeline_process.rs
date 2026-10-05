//! Shared real-process fixture for Gate 10 tests and acceptance demo.
#![allow(dead_code)]
use agend_core::model::Backend;
use agend_core::pipeline::workflow::{
    Approver, Stage, WorkOutput, Workflow, WorkflowRequirement, WorkflowStage,
};
use agend_core::protocol::client::*;
use agend_daemon::store::{Instance, InstanceStatus, SqliteStore, pipeline::Team};
use agend_testkit::block_on;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

pub fn home() -> Result<PathBuf, String> {
    let h = std::env::var_os("AGEND_HOME").ok_or("AGEND_HOME is not set")?;
    if !Path::new(&h).is_absolute() {
        return Err("AGEND_HOME must be absolute".into());
    }
    Ok(h.into())
}
pub fn binaries() -> Result<(PathBuf, PathBuf), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let parent = exe.parent().ok_or("no executable directory")?;
    let target = if parent.ends_with("examples") || parent.ends_with("deps") {
        parent.parent().ok_or("no target directory")?
    } else {
        parent
    };
    let agend = std::env::var_os("AGEND_BIN").map_or(target.join("agend"), PathBuf::from);
    let worker =
        std::env::var_os("AGEND_WORKER_BIN").map_or(target.join("fake-worker"), PathBuf::from);
    if !agend.is_file() || !worker.is_file() {
        return Err(format!(
            "build first: cargo build -p agend -p agend-testkit --bins ({}; {})",
            agend.display(),
            worker.display()
        ));
    }
    Ok((agend, worker))
}
pub fn workflow(id: &str, command: &str) -> Workflow {
    Workflow {
        id: id.into(),
        version: 1,
        requires: vec![WorkflowRequirement::Repo],
        allow_unreviewed: false,
        stages: vec![
            WorkflowStage::new(
                "work",
                Stage::Work {
                    role: "dev".into(),
                    instructions: "Commit hello.txt".into(),
                    output: WorkOutput::Branch,
                },
            ),
            WorkflowStage::new(
                "submit",
                Stage::Submit {
                    forge: "local".into(),
                },
            ),
            WorkflowStage {
                timeout_ms: Some(if id == "slow" { 120_000 } else { 60_000 }),
                ..WorkflowStage::new(
                    "checks",
                    Stage::Command {
                        command: command.into(),
                    },
                )
            },
            WorkflowStage::new(
                "review",
                Stage::Approval {
                    by: Approver::Role("reviewer".into()),
                    count: 1,
                    bind_head: true,
                },
            ),
            WorkflowStage::new(
                "approve",
                Stage::Approval {
                    by: Approver::Human,
                    count: 1,
                    bind_head: true,
                },
            ),
            WorkflowStage::new("merge", Stage::Merge),
        ],
    }
}
pub fn setup(home: &Path, flags: &[&str]) -> Result<(), String> {
    let (_, worker) = binaries()?;
    std::fs::create_dir_all(home).map_err(|e| e.to_string())?;
    let home = home.canonicalize().map_err(|e| e.to_string())?;
    let repo = PathBuf::from(format!("{}-repo", home.display()));
    if repo.exists() {
        return Err("fixture repo already exists; use a fresh home".into());
    }
    std::fs::create_dir_all(&repo).map_err(|e| e.to_string())?;
    for args in [
        vec!["init", "-b", "main"],
        vec!["config", "user.name", "Agend pipeline fixture"],
        vec!["config", "user.email", "fixture@example.invalid"],
    ] {
        git(&repo, &args)?;
    }
    std::fs::write(repo.join("README.md"), "Pipeline fixture\n").map_err(|e| e.to_string())?;
    git(&repo, &["add", "README.md"])?;
    git(&repo, &["commit", "-m", "Initial fixture"])?;
    let store = SqliteStore::open(&home, 0).map_err(|e| e.to_string())?;
    for w in [
        workflow("demo", "test -f hello.txt"),
        workflow("slow", "sleep 20; test -f hello.txt"),
        workflow(
            "escape",
            &format!(
                "touch {} && test -f hello.txt",
                agend_daemon::runner::quote(&repo.join("escaped").display().to_string())
            ),
        ),
    ] {
        block_on(store.save_workflow(&w)).map_err(|e| e.to_string())?;
    }
    for id in ["g10", "g10h"] {
        block_on(store.add_team(&Team {
            id: id.into(),
            repo: Some(repo.display().to_string()),
            default_workflow: "demo".into(),
        }))
        .map_err(|e| e.to_string())?;
    }
    for (id, team, role) in [
        ("g10-dev", "g10", "dev"),
        ("g10-rev", "g10", "reviewer"),
        ("g10-hold", "g10h", "dev"),
    ] {
        let workspace = home.join("workspace").join(id);
        std::fs::create_dir_all(&workspace).map_err(|e| e.to_string())?;
        let args = if id == "g10-hold" {
            vec!["--hold".into()]
        } else {
            flags
                .iter()
                .filter(|f| {
                    if id == "g10-rev" {
                        **f == "--changes-once"
                    } else {
                        **f != "--changes-once"
                    }
                })
                .map(|f| f.to_string())
                .collect()
        };
        let instance = Instance {
            id: id.into(),
            backend: Backend::Claude,
            program: worker.display().to_string(),
            args,
            working_directory: workspace.display().to_string(),
            session_id: Some(
                agend_daemon::store::instances::new_session_id().map_err(|e| e.to_string())?,
            ),
            status: InstanceStatus::New,
            session_started: false,
            agent_pid: None,
            legacy_no_thread: false,
            delivery: "inbox".into(),
        };
        block_on(store.add_instance(&instance)).map_err(|e| e.to_string())?;
        block_on(store.join_team(team, id, role)).map_err(|e| e.to_string())?;
        block_on(store.set_inbox_delivery(id)).map_err(|e| e.to_string())?;
    }
    println!(
        "repo={}\nteam g10: dev g10-dev, reviewer g10-rev\nteam g10h: dev g10-hold (--hold)\nworkflow demo v1: work -> submit -> checks -> review -> approve(human) -> merge",
        repo.display()
    );
    Ok(())
}
pub fn git(repo: &Path, args: &[&str]) -> Result<String, String> {
    let mut cmd = Command::new("git");
    cmd.current_dir(repo)
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "core.fsmonitor=false",
        ])
        .args(args);
    for (k, _) in std::env::vars().filter(|(k, _)| k.starts_with("GIT_") || k.starts_with("AGEND_"))
    {
        cmd.env_remove(k);
    }
    let o = cmd.output().map_err(|e| e.to_string())?;
    if !o.status.success() {
        return Err(String::from_utf8_lossy(&o.stderr).into());
    }
    Ok(String::from_utf8_lossy(&o.stdout).trim().into())
}
pub fn teardown(home: &Path) -> Result<(), String> {
    let ids = if home.join("agend.db").exists() {
        let store = SqliteStore::open(home, 0).map_err(|e| e.to_string())?;
        block_on(store.instances())
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|i| i.id)
            .collect::<Vec<_>>()
    } else {
        vec![]
    };
    for id in ids {
        let _ = agend_daemon::runtime::shutdown_holder_within(home, &id, Duration::from_secs(5));
    }
    std::fs::remove_dir_all(home).map_err(|e| e.to_string())?;
    let repo = PathBuf::from(format!("{}-repo", home.display()));
    if repo.exists() {
        std::fs::remove_dir_all(repo).map_err(|e| e.to_string())?;
    }
    Ok(())
}
pub struct Lab {
    pub home: PathBuf,
    pub daemon: Option<Child>,
    pub agend: PathBuf,
    pub environment: Vec<(String, String)>,
}
impl Lab {
    pub fn new(flags: &[&str]) -> Result<Self, String> {
        let (agend, _) = binaries()?;
        let home = PathBuf::from("/tmp").join(format!(
            "g10-{}",
            &agend_daemon::store::instances::new_session_id().map_err(|e| e.to_string())?[..8]
        ));
        std::fs::create_dir(&home).map_err(|e| e.to_string())?;
        let home = home.canonicalize().map_err(|e| e.to_string())?;
        setup(&home, flags)?;
        Ok(Self {
            home,
            daemon: None,
            agend,
            environment: Vec::new(),
        })
    }
    pub fn boot(&mut self, failpoint: Option<&str>) -> Result<(), String> {
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.home.join("process.log"))
            .map_err(|e| e.to_string())?;
        let mut cmd = Command::new(&self.agend);
        cmd.arg("daemon")
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    self.agend
                        .parent()
                        .unwrap_or(Path::new("/usr/bin"))
                        .display(),
                    std::env::var("PATH").unwrap_or_default()
                ),
            )
            .env("AGEND_HOME", &self.home)
            .env_remove("AGEND_INSTANCE")
            .env_remove("AGEND_FAILPOINT")
            .stdout(Stdio::from(log.try_clone().map_err(|e| e.to_string())?))
            .stderr(Stdio::from(log));
        if let Some(f) = failpoint {
            cmd.env("AGEND_FAILPOINT", f);
        }
        cmd.envs(self.environment.clone());
        self.daemon = Some(cmd.spawn().map_err(|e| e.to_string())?);
        let until = Instant::now() + Duration::from_secs(25);
        while Instant::now() < until {
            if let Some(child) = &mut self.daemon
                && child.try_wait().map_err(|e| e.to_string())?.is_some()
            {
                return Err(format!("daemon exited at boot: {}", self.logs()));
            }
            if self.fleet().is_ok() {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        Err(format!("daemon did not boot: {}", self.logs()))
    }
    pub fn logs(&self) -> String {
        std::fs::read_to_string(self.home.join("process.log")).unwrap_or_default()
    }
    pub fn stop(&mut self, hard: bool) {
        if let Some(mut child) = self.daemon.take() {
            if hard {
                let _ = child.kill();
            } else {
                let pid = child.id();
                if pid > 1 {
                    unsafe {
                        libc::kill(pid as i32, libc::SIGINT);
                    }
                }
            }
            let _ = child.wait();
        }
    }
    pub fn request(
        &self,
        caller: Option<&str>,
        request: ClientRequest,
    ) -> Result<ClientResponse, String> {
        let stream =
            UnixStream::connect(self.home.join(DAEMON_SOCKET)).map_err(|e| e.to_string())?;
        stream
            .set_read_timeout(Some(Duration::from_secs(30)))
            .map_err(|e| e.to_string())?;
        let mut reader = BufReader::new(stream);
        fn send(
            reader: &mut BufReader<UnixStream>,
            r: &ClientRequest,
        ) -> Result<ClientResponse, String> {
            let text = serde_json::to_string(r).map_err(|e| e.to_string())?;
            writeln!(reader.get_mut(), "{text}").map_err(|e| e.to_string())?;
            let mut line = String::new();
            reader.read_line(&mut line).map_err(|e| e.to_string())?;
            serde_json::from_str(&line).map_err(|e| e.to_string())
        }
        send(
            &mut reader,
            &ClientRequest::hello_as(caller.map(str::to_owned)),
        )?;
        match send(&mut reader, &request)? {
            ClientResponse::Error { data } => Err(format!("{}: {}", data.code, data.message)),
            r => Ok(r),
        }
    }
    pub fn operator(&self, command: OperatorCommand) -> Result<CommandResult, String> {
        match self.request(
            None,
            ClientRequest::Operator {
                data: OperatorData {
                    request_id: "test".into(),
                    command,
                },
            },
        )? {
            ClientResponse::CommandResult { data } => Ok(data.result),
            r => Err(format!("unexpected {r:?}")),
        }
    }
    pub fn agent(&self, caller: &str, command: AgentCommand) -> Result<CommandResult, String> {
        match self.request(
            Some(caller),
            ClientRequest::Command {
                data: ClientCommandData {
                    request_id: "agent-test".into(),
                    command,
                },
            },
        )? {
            ClientResponse::CommandResult { data } => Ok(data.result),
            other => Err(format!("unexpected {other:?}")),
        }
    }
    pub fn fleet(&self) -> Result<FleetView, String> {
        match self.request(
            None,
            ClientRequest::GetFleet {
                data: RequestIdData {
                    request_id: "view".into(),
                },
            },
        )? {
            ClientResponse::Fleet { data } => Ok(data.fleet),
            r => Err(format!("unexpected {r:?}")),
        }
    }
    pub fn create(&self, team: &str, workflow: &str, title: &str) -> Result<String, String> {
        match self.operator(OperatorCommand::TaskCreate {
            title: title.into(),
            role: "dev".into(),
            team_id: team.into(),
            workflow_id: Some(workflow.into()),
        })? {
            CommandResult::TaskCreated { data } => Ok(data.task_id),
            r => Err(format!("unexpected {r:?}")),
        }
    }
    fn wait_view(
        &self,
        expected: &str,
        ready: impl Fn(&FleetView) -> bool,
    ) -> Result<FleetView, String> {
        let until = Instant::now() + Duration::from_secs(60);
        while Instant::now() < until {
            let view = self.fleet()?;
            if ready(&view) {
                return Ok(view);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        Err(format!("never observed {expected}: {}", self.logs()))
    }
    pub fn wait_stage(&self, task: &str, stage: &str) -> Result<FleetView, String> {
        self.wait_view(&format!("{task} at {stage}"), |view| {
            view.tasks.iter().any(|t| {
                t.task_id == task
                    && (t.current_stage.as_deref() == Some(stage) || t.status == stage)
            })
        })
    }
    pub fn approve(&self, task: &str) -> Result<(), String> {
        // A stage transition can be visible before its attention is published.
        // Resolve only after one native FleetView contains both prerequisites.
        let approval = |view: &FleetView| {
            view.attention
                .iter()
                .find(|a| {
                    a.task_id.as_deref() == Some(task)
                        && a.actions.contains(&AttentionAction::Approve)
                        && a.attention_id.is_some()
                })
                .and_then(|a| a.attention_id.clone())
        };
        let view = self.wait_view(&format!("{task} approval attention"), |view| {
            view.tasks
                .iter()
                .any(|t| t.task_id == task && t.current_stage.as_deref() == Some("approve"))
                && approval(view).is_some()
        })?;
        let id = approval(&view).ok_or("approval attention missing")?;
        self.request(
            None,
            ClientRequest::ResolveAttention {
                data: ResolveAttentionData {
                    request_id: "approve".into(),
                    attention_id: id,
                    action: AttentionAction::Approve,
                    note: None,
                },
            },
        )?;
        Ok(())
    }
    pub fn repo(&self) -> PathBuf {
        PathBuf::from(format!("{}-repo", self.home.display()))
    }
}
impl Drop for Lab {
    fn drop(&mut self) {
        self.stop(false);
        let _ = teardown(&self.home);
    }
}

pub fn happy() -> Result<(), String> {
    let mut lab = Lab::new(&[])?;
    lab.boot(None)?;
    let task = lab.create("g10", "demo", "hello")?;
    lab.approve(&task)?;
    lab.wait_stage(&task, "done")?;
    let log = git(&lab.repo(), &["log", "--format=%B", "main"])?;
    if log.matches(&format!("Agend-Task: {task}")).count() != 1 {
        return Err("merge count is not one".into());
    }
    println!("{task}: checks passed; review approved; human approved; done; merge commits=1");
    Ok(())
}
pub fn demo() -> Result<(), String> {
    println!("== happy");
    happy()?;
    for (section, flag) in [
        ("checks-fail", "--fail-checks-once"),
        ("changes", "--changes-once"),
        ("wip", "--leave-wip"),
    ] {
        println!("== {section}");
        rework(flag)?;
    }
    println!("== main-advanced");
    main_advanced()?;
    for failpoint in ["after-merge-intent", "after-main-moved"] {
        println!("== restart: {failpoint}");
        four_boots(failpoint)?;
    }
    fresh_home_negative()?;
    println!("== sandbox");
    sandbox_escape()?;
    println!("== hooks");
    hooks_and_cancel()?;
    println!("pipeline demo: all sections passed");
    Ok(())
}

pub fn wait_until(lab: &Lab, predicate: impl Fn() -> Result<bool, String>) -> Result<(), String> {
    let until = Instant::now() + Duration::from_secs(60);
    while Instant::now() < until {
        if predicate()? {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Err(format!("condition did not become true: {}", lab.logs()))
}
pub fn database(lab: &Lab) -> Result<rusqlite::Connection, String> {
    let c = rusqlite::Connection::open(lab.home.join("agend.db")).map_err(|e| e.to_string())?;
    c.busy_timeout(Duration::from_secs(5))
        .map_err(|e| e.to_string())?;
    Ok(c)
}
fn assert_cleanup(lab: &mut Lab, task: &str) -> Result<(), String> {
    wait_until(lab, || {
        Ok(lab
            .fleet()?
            .tasks
            .iter()
            .any(|t| t.task_id == task && t.assignee.is_none())
            && !lab.home.join("worktrees").join(task).exists())
    })?;
    lab.stop(false);
    let count: i64 = database(lab)?
        .query_row(
            "SELECT count(*) FROM bindings WHERE task_id=?1",
            [task],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if count != 0 {
        return Err("binding survived cleanup".into());
    }
    if !git(
        &lab.repo(),
        &[
            "for-each-ref",
            "--format=%(refname)",
            &format!("refs/heads/agend/{task}/"),
        ],
    )?
    .is_empty()
    {
        return Err("work branch survived completion".into());
    }
    Ok(())
}
pub fn rework(flag: &str) -> Result<(), String> {
    let mut lab = Lab::new(&[flag])?;
    lab.boot(None)?;
    let task = lab.create("g10", "demo", "rework")?;
    lab.approve(&task)?;
    lab.wait_stage(&task, "done")?;
    assert_cleanup(&mut lab, &task)?;
    let snapshot: String = database(&lab)?
        .query_row("SELECT pipeline FROM tasks WHERE id=?1", [&task], |r| {
            r.get(0)
        })
        .map_err(|e| e.to_string())?;
    let json: serde_json::Value = serde_json::from_str(&snapshot).map_err(|e| e.to_string())?;
    if flag != "--leave-wip" && json["attempts"][0].as_u64() != Some(2) {
        return Err(format!("rework did not enter attempt 2: {snapshot}"));
    }
    if flag == "--leave-wip" {
        let patches = std::fs::read_dir(lab.home.join("archive"))
            .map_err(|e| e.to_string())?
            .flatten()
            .filter_map(|p| std::fs::read_to_string(p.path()).ok())
            .collect::<Vec<_>>()
            .join("\n");
        if !patches.contains("unfinished.txt") || !patches.contains("uncommitted work") {
            return Err("WIP was not archived".into());
        }
    }
    println!("{task}: {flag} passed; binding, branch and worktree removed");
    Ok(())
}
pub fn main_advanced() -> Result<(), String> {
    let mut lab = Lab::new(&[])?;
    lab.boot(None)?;
    let task = lab.create("g10", "demo", "main advanced")?;
    lab.wait_stage(&task, "approve")?;
    std::fs::write(lab.repo().join("main-only.txt"), "new main\n").map_err(|e| e.to_string())?;
    git(&lab.repo(), &["add", "main-only.txt"])?;
    git(&lab.repo(), &["commit", "-m", "Advance main"])?;
    let advanced = git(&lab.repo(), &["rev-parse", "main"])?;
    lab.approve(&task)?;
    lab.wait_stage(&task, "done")?;
    assert_cleanup(&mut lab, &task)?;
    let c = database(&lab)?;
    let snapshot: String = c
        .query_row("SELECT pipeline FROM tasks WHERE id=?1", [&task], |r| {
            r.get(0)
        })
        .map_err(|e| e.to_string())?;
    let json: serde_json::Value = serde_json::from_str(&snapshot).map_err(|e| e.to_string())?;
    if json["attempts"][2] != 2 || json["attempts"][3] != 1 || json["attempts"][4] != 1 {
        return Err(format!(
            "same-patch rebase did not preserve approvals: {snapshot}"
        ));
    }
    if git(&lab.repo(), &["rev-parse", "main^1"])? != advanced {
        return Err("merge does not include advanced main as first parent".into());
    }
    println!("{task}: approvals kept; checks attempt 2; merge includes latest main");
    Ok(())
}
pub fn four_boots(failpoint: &str) -> Result<(), String> {
    let mut lab = Lab::new(&[])?;
    lab.boot(None)?;
    let task = lab.create("g10", "slow", "four boots")?;
    lab.wait_stage(&task, "checks")?;
    println!(
        "boot 1 daemon pid={}: {task}/checks/1 running; kill -9",
        lab.daemon.as_ref().unwrap().id()
    );
    lab.stop(true);
    lab.boot(None)?;
    let view = lab.wait_stage(&task, "approve")?;
    println!(
        "boot 2 daemon pid={}: {task}: re-running checks; review approved; waiting for approve",
        lab.daemon.as_ref().unwrap().id()
    );
    let waiting = view
        .attention
        .iter()
        .find(|a| a.task_id.as_deref() == Some(&task))
        .and_then(|a| a.waiting_since_unix_ms)
        .ok_or("attention missing")?;
    lab.stop(false);
    lab.boot(Some(failpoint))?;
    println!(
        "boot 3 daemon pid={}: failpoint {failpoint}",
        lab.daemon.as_ref().unwrap().id()
    );
    let view = lab.wait_stage(&task, "approve")?;
    if view
        .attention
        .iter()
        .find(|a| a.task_id.as_deref() == Some(&task))
        .and_then(|a| a.waiting_since_unix_ms)
        != Some(waiting)
    {
        return Err("restart reset waiting time".into());
    }
    let _ = lab.approve(&task);
    let until = Instant::now() + Duration::from_secs(30);
    loop {
        if lab
            .daemon
            .as_mut()
            .ok_or("daemon missing")?
            .try_wait()
            .map_err(|e| e.to_string())?
            .is_some()
        {
            break;
        }
        if Instant::now() > until {
            return Err(format!("failpoint never fired: {}", lab.logs()));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    lab.stop(true);
    if failpoint == "after-main-moved" {
        // Simulate loss of the last DB checkpoint: the trailer is the remaining proof.
        database(&lab)?
            .execute("UPDATE tasks SET merge_intent=NULL WHERE id=?1", [&task])
            .map_err(|e| e.to_string())?;
    }
    lab.boot(None)?;
    lab.wait_stage(&task, "done")?;
    println!(
        "boot 4 daemon pid={}: {task}: merge found on main; not merged again",
        lab.daemon.as_ref().unwrap().id()
    );
    assert_cleanup(&mut lab, &task)?;
    let log = git(&lab.repo(), &["log", "--format=%B", "main"])?;
    if log.matches(&format!("Agend-Task: {task}")).count() != 1 {
        return Err("restart made duplicate merge commits".into());
    }
    let c = database(&lab)?;
    let dispatches: i64 = c
        .query_row(
            "SELECT count(*) FROM messages WHERE task_id=?1 AND id LIKE 'dispatch:%'",
            [&task],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if dispatches != 2 {
        return Err(format!(
            "expected one work and one review dispatch, got {dispatches}"
        ));
    }
    println!(
        "{task}: four boots; {failpoint}; merge commits=1; no duplicate dispatch; cleanup complete"
    );
    Ok(())
}
pub fn sandbox_escape() -> Result<(), String> {
    let mut lab = Lab::new(&[])?;
    lab.boot(None)?;
    let task = lab.create("g10", "escape", "sandbox escape")?;
    wait_until(&lab, || {
        Ok(lab.logs().contains("checks") && lab.logs().contains("failed"))
    })?;
    if lab.repo().join("escaped").exists() {
        return Err("checks escaped the sandbox".into());
    }
    lab.operator(OperatorCommand::TaskCancel {
        task_id: task.clone(),
        reason: None,
    })?;
    lab.wait_stage(&task, "cancelled")?;
    println!("{task}: outside write denied; task never approved or merged");
    Ok(())
}
pub fn hooks_and_cancel() -> Result<(), String> {
    let mut lab = Lab::new(&[])?;
    lab.boot(None)?;
    let task = lab.create("g10h", "demo", "hooks")?;
    let wt = lab.home.join("worktrees").join(&task);
    wait_until(&lab, || {
        Ok(lab.home.join("bindings/g10-hold.json").exists() && wt.join(".git").exists())
    })?;
    let o = Command::new(lab.home.join("bin/git"))
        .args(["checkout", "main"])
        .current_dir(&wt)
        .env("AGEND_HOME", &lab.home)
        .env("AGEND_INSTANCE", "g10-hold")
        .output()
        .map_err(|e| e.to_string())?;
    if o.status.success() {
        return Err("shim allowed checking out main".into());
    }
    git(
        &wt,
        &["commit", "--allow-empty", "-m", "Hook probe on task branch"],
    )?;
    let main = git(&lab.repo(), &["rev-parse", "main"])?;
    let o = Command::new("git")
        .args(["update-ref", "refs/heads/main", "HEAD"])
        .current_dir(&wt)
        .output()
        .map_err(|e| e.to_string())?;
    if o.status.success()
        || !String::from_utf8_lossy(&o.stderr).contains("reference-transaction hook")
    {
        return Err(format!(
            "reference hook did not refuse main update: {}",
            String::from_utf8_lossy(&o.stderr)
        ));
    }
    assert_eq!(git(&lab.repo(), &["rev-parse", "main"])?, main);
    std::fs::write(wt.join("lost.txt"), "keep this WIP\n").map_err(|e| e.to_string())?;
    lab.operator(OperatorCommand::TaskCancel {
        task_id: task.clone(),
        reason: Some("acceptance cancellation".into()),
    })?;
    lab.wait_stage(&task, "cancelled")?;
    assert_cleanup(&mut lab, &task)?;
    let snap: serde_json::Value = serde_json::from_slice(
        &std::fs::read(lab.home.join("bindings/g10-hold.json")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    if !snap["binding"].is_null() {
        return Err("released snapshot remains bound".into());
    }
    println!("{task}: checkout main refused; cancel archived WIP; snapshot unbound");
    Ok(())
}

pub fn commands_and_dialogue() -> Result<(), String> {
    use agend_core::protocol::ask::{AnswerSource, AskReply};
    let mut lab = Lab::new(&[])?;
    lab.boot(None)?;
    let task = lab.create("g10h", "demo", "blocked task")?;
    wait_until(&lab, || {
        Ok(lab
            .fleet()?
            .tasks
            .iter()
            .any(|t| t.task_id == task && t.assignee.as_deref() == Some("g10-hold")))
    })?;
    let done = AgentCommand::Done {
        task_id: task.clone(),
        identity: Some(ResultIdentity {
            stage_id: "work".into(),
            attempt: 1,
        }),
    };
    assert!(
        lab.agent("g10-hold", done.clone())
            .unwrap_err()
            .contains("nothing to merge")
    );
    assert!(
        lab.agent("g10-dev", done)
            .unwrap_err()
            .contains("forbidden")
    );
    assert!(
        lab.operator(OperatorCommand::InstanceRemove {
            instance_id: "g10-hold".into()
        })
        .unwrap_err()
        .contains("cancel")
    );
    lab.agent(
        "g10-hold",
        AgentCommand::Block {
            task_id: task.clone(),
            reason: "API key required".into(),
        },
    )?;
    let view = lab.wait_stage(&task, "blocked")?;
    assert_eq!(
        view.tasks
            .iter()
            .find(|t| t.task_id == task)
            .and_then(|t| t.pipeline.as_ref())
            .and_then(|p| p.block_reason.as_deref()),
        Some("API key required")
    );
    assert!(
        !view
            .attention
            .iter()
            .any(|a| a.task_id.as_deref() == Some(&task))
    );
    lab.agent(
        "g10-hold",
        AgentCommand::Remind {
            task_id: task.clone(),
            delay_seconds: 2,
        },
    )?;
    let ask = match lab.agent(
        "g10-hold",
        AgentCommand::Ask {
            question: "Which API?".into(),
            options: vec!["stable".into()],
        },
    )? {
        CommandResult::AskCreated { data } => data.ask_id,
        other => return Err(format!("unexpected {other:?}")),
    };
    lab.request(
        None,
        ClientRequest::AnswerAsk {
            data: AnswerAskData {
                request_id: "answer".into(),
                ask_id: ask.clone(),
                source: AnswerSource::Cli,
                reply: AskReply::Text {
                    text: "Stable API".into(),
                },
            },
        },
    )?;
    lab.stop(true);
    lab.boot(None)?;
    lab.agent(
        "g10-hold",
        AgentCommand::AskFollowUp {
            ask_id: ask.clone(),
            question: "Which version?".into(),
            options: vec![],
        },
    )?;
    lab.request(
        None,
        ClientRequest::AnswerAsk {
            data: AnswerAskData {
                request_id: "answer-2".into(),
                ask_id: ask.clone(),
                source: AnswerSource::Cli,
                reply: AskReply::Text {
                    text: "Version 2".into(),
                },
            },
        },
    )?;
    lab.agent(
        "g10-hold",
        AgentCommand::AskResolve {
            ask_id: ask.clone(),
            summary: "Stable API v2".into(),
        },
    )?;
    assert!(
        lab.agent(
            "g10-hold",
            AgentCommand::AskFollowUp {
                ask_id: ask.clone(),
                question: "stale?".into(),
                options: vec![]
            }
        )
        .is_err()
    );
    wait_until(&lab, || {
        Ok(
            matches!(lab.agent("g10-hold",AgentCommand::Inbox { after_message_id:None })?, CommandResult::Messages { data } if data.messages.iter().any(|m| m.message_id.starts_with("remind:"))),
        )
    })?;
    lab.agent(
        "g10-hold",
        AgentCommand::Unblock {
            task_id: task.clone(),
        },
    )?;
    lab.operator(OperatorCommand::TaskCancel {
        task_id: task.clone(),
        reason: None,
    })?;
    lab.wait_stage(&task, "cancelled")?;
    assert_cleanup(&mut lab, &task)?;
    let c = database(&lab)?;
    let turns: i64 = c
        .query_row(
            "SELECT count(*) FROM ask_turns WHERE ask_id=?1",
            [ask],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    assert_eq!(turns, 5);
    let remaining: i64 = c
        .query_row("SELECT count(*) FROM reminders", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    assert_eq!(remaining, 0);
    println!("{task}: ownership, empty branch, block/unblock, reminder and five-turn ask passed");
    Ok(())
}
pub fn queue_and_role_join() -> Result<(), String> {
    let mut lab = Lab::new(&[])?;
    lab.boot(None)?;
    lab.operator(OperatorCommand::TeamJoin {
        team_id: "g10h".into(),
        instance_id: "g10-rev".into(),
        role: "other".into(),
    })?;
    let first = lab.create("g10", "demo", "first")?;
    let second = lab.create("g10", "demo", "queued")?;
    let view = lab.wait_stage(&first, "review")?;
    assert!(
        view.tasks
            .iter()
            .any(|t| t.task_id == second && t.assignee.is_none())
    );
    wait_until(&lab, || {
        Ok(lab
            .fleet()?
            .attention
            .iter()
            .any(|a| a.attention_id.as_deref() == Some("no-role:g10/reviewer")))
    })?;
    lab.operator(OperatorCommand::TeamJoin {
        team_id: "g10".into(),
        instance_id: "g10-rev".into(),
        role: "reviewer".into(),
    })?;
    lab.approve(&first)?;
    lab.wait_stage(&first, "done")?;
    lab.approve(&second)?;
    lab.wait_stage(&second, "done")?;
    assert!(
        !lab.fleet()?
            .attention
            .iter()
            .any(|a| a.attention_id.as_deref() == Some("no-role:g10/reviewer"))
    );
    println!("{first}, {second}: busy holder queues; role join automatically resumes review");
    Ok(())
}

pub fn sandbox_retry() -> Result<(), String> {
    let mut lab = Lab::new(&[])?;
    let missing = lab.home.join("sandbox-tool");
    lab.environment
        .push(("AGEND_SANDBOX_TOOL".into(), missing.display().to_string()));
    lab.boot(None)?;
    let task = lab.create("g10", "demo", "sandbox retry")?;
    let id = format!("sandbox-missing:{task}");
    wait_until(&lab, || {
        Ok(lab
            .fleet()?
            .attention
            .iter()
            .any(|a| a.attention_id.as_deref() == Some(&id)))
    })?;
    assert!(
        lab.fleet()?
            .tasks
            .iter()
            .any(|t| t.task_id == task && t.current_stage.as_deref() == Some("checks"))
    );
    let tool = agend_daemon::checks::tool(&lab.home).ok_or("platform sandbox missing")?;
    std::os::unix::fs::symlink(tool, &missing).map_err(|e| e.to_string())?;
    lab.request(
        None,
        ClientRequest::ResolveAttention {
            data: ResolveAttentionData {
                request_id: "retry".into(),
                attention_id: id,
                action: AttentionAction::Retry,
                note: None,
            },
        },
    )?;
    lab.approve(&task)?;
    lab.wait_stage(&task, "done")?;
    println!("{task}: missing tool failed closed; retry after restoring tool completed");
    Ok(())
}
pub fn merge_blocked() -> Result<(), String> {
    let mut lab = Lab::new(&[])?;
    lab.boot(None)?;
    let task = lab.create("g10", "demo", "blocked merge")?;
    lab.wait_stage(&task, "approve")?;
    std::fs::write(lab.repo().join("dirty-main"), "operator WIP\n").map_err(|e| e.to_string())?;
    let before = git(&lab.repo(), &["rev-parse", "main"])?;
    lab.approve(&task)?;
    let id = format!("merge-blocked:{task}");
    wait_until(&lab, || {
        Ok(lab
            .fleet()?
            .attention
            .iter()
            .any(|a| a.attention_id.as_deref() == Some(&id)))
    })?;
    assert_eq!(git(&lab.repo(), &["rev-parse", "main"])?, before);
    assert!(
        lab.operator(OperatorCommand::TaskCancel {
            task_id: task.clone(),
            reason: None
        })
        .unwrap_err()
        .contains("merge_in_flight")
    );
    std::fs::remove_file(lab.repo().join("dirty-main")).map_err(|e| e.to_string())?;
    lab.request(
        None,
        ClientRequest::ResolveAttention {
            data: ResolveAttentionData {
                request_id: "retry".into(),
                attention_id: id,
                action: AttentionAction::Retry,
                note: None,
            },
        },
    )?;
    lab.wait_stage(&task, "done")?;
    println!("{task}: dirty main blocked merge and cancellation; retry completed after cleanup");
    Ok(())
}

pub fn malformed_restore() -> Result<(), String> {
    let mut lab = Lab::new(&[])?;
    lab.boot(None)?;
    let task = lab.create("g10h", "demo", "malformed snapshot")?;
    wait_until(&lab, || {
        Ok(lab.home.join("worktrees").join(&task).join(".git").exists())
    })?;
    lab.stop(true);
    let c = database(&lab)?;
    let text: String = c
        .query_row("SELECT pipeline FROM tasks WHERE id=?1", [&task], |r| {
            r.get(0)
        })
        .map_err(|e| e.to_string())?;
    let mut json: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    json["stage_index"] = serde_json::json!(999);
    c.execute(
        "UPDATE tasks SET pipeline=?1 WHERE id=?2",
        rusqlite::params![json.to_string(), task],
    )
    .map_err(|e| e.to_string())?;
    drop(c);
    lab.boot(None)?;
    lab.wait_stage(&task, "failed")?;
    let id = format!("task-failed:{task}");
    wait_until(&lab, || {
        Ok(lab
            .fleet()?
            .attention
            .iter()
            .any(|a| a.attention_id.as_deref() == Some(&id)))
    })?;
    lab.request(
        None,
        ClientRequest::ResolveAttention {
            data: ResolveAttentionData {
                request_id: "ack".into(),
                attention_id: id.clone(),
                action: AttentionAction::Acknowledge,
                note: None,
            },
        },
    )?;
    lab.stop(false);
    lab.boot(None)?;
    assert!(
        lab.fleet()?
            .tasks
            .iter()
            .any(|t| t.task_id == task && t.status == "failed")
    );
    assert!(
        !lab.fleet()?
            .attention
            .iter()
            .any(|a| a.attention_id.as_deref() == Some(&id))
    );
    let healthy = lab.create("g10h", "demo", "healthy")?;
    wait_until(&lab, || {
        Ok(lab
            .fleet()?
            .tasks
            .iter()
            .any(|t| t.task_id == healthy && t.assignee.as_deref() == Some("g10-hold")))
    })?;
    lab.operator(OperatorCommand::TaskCancel {
        task_id: healthy,
        reason: None,
    })?;
    println!("{task}: malformed snapshot isolated; acknowledgment persists; other tasks run");
    Ok(())
}
pub fn fresh_home_negative() -> Result<(), String> {
    let mut first = Lab::new(&[])?;
    first.boot(None)?;
    let task = first.create("g10h", "demo", "old home task")?;
    first.stop(true);
    let mut fresh = Lab::new(&[])?;
    fresh.boot(None)?;
    assert!(fresh.fleet()?.tasks.is_empty());
    assert!(
        fresh
            .operator(OperatorCommand::TaskCancel {
                task_id: task.clone(),
                reason: None
            })
            .unwrap_err()
            .contains("unknown task")
    );
    println!("negative check (new AGEND_HOME): boot 2 failed: {task} unknown");
    Ok(())
}
