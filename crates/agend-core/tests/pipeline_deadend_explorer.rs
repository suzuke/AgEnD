//! Dead-end explorer written by the fresh-context gate 1 verifier (round r3),
//! ported as an ignored deep test. An independent PCG32 workflow generator
//! feeds `validate`; for each accepted workflow a bounded breadth-first search
//! over head changes, failures and stale, duplicate or timeout events checks
//! safety invariants and that from every reachable non-terminal state a
//! success-only continuation still reaches done.
//!
//! Run in release (about 2 minutes): `cargo test --release -p agend-core --test
//! pipeline_deadend_explorer -- --ignored` (`A4R3_N`, `A4R3_EXPLORE`,
//! `A4R3_CAP` change the budget).
#![allow(clippy::all, dead_code)]

use agend_core::pipeline::stage::{FanoutJoin, StageKind};
use agend_core::pipeline::state::*;
use agend_core::pipeline::workflow::*;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::panic::{AssertUnwindSafe, catch_unwind};

struct Pcg(u64, u64);
impl Pcg {
    fn new(seed: u64, stream: u64) -> Self {
        let mut p = Pcg(0, (stream << 1) | 1);
        p.next();
        p.0 = p.0.wrapping_add(seed);
        p.next();
        p
    }
    fn next(&mut self) -> u32 {
        let old = self.0;
        self.0 = old.wrapping_mul(6364136223846793005).wrapping_add(self.1);
        let xs = (((old >> 18) ^ old) >> 27) as u32;
        xs.rotate_right((old >> 59) as u32)
    }
    fn below(&mut self, n: u32) -> u32 {
        self.next() % n
    }
    fn pct(&mut self, p: u32) -> bool {
        self.below(100) < p
    }
}

fn roles() -> Vec<String> {
    ["dev", "reviewer", "planner", "qa", "researcher"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

fn mk_work(r: &mut Pcg, out: WorkOutput) -> Stage {
    Stage::Work {
        role: ["dev", "planner", "qa", "dev"][r.below(4) as usize].into(),
        instructions: "x".into(),
        output: out,
    }
}
fn mk_cmd(r: &mut Pcg) -> Stage {
    Stage::Command {
        command: [
            "make check",
            "gh pr checks {pr} --watch",
            "git diff {head}",
            "echo {branch} {head}",
            "true",
        ][r.below(5) as usize]
            .into(),
    }
}
fn mk_appr(r: &mut Pcg, bind: bool) -> Stage {
    Stage::Approval {
        by: if r.pct(40) {
            Approver::Human
        } else {
            Approver::Role(["reviewer", "qa"][r.below(2) as usize].into())
        },
        count: 1 + r.below(3) as u8,
        bind_head: bind,
    }
}
fn mk_fanout(r: &mut Pcg, allow_plan: bool) -> Stage {
    let source = if allow_plan && r.pct(50) {
        FanoutSource::WorkOutput
    } else {
        FanoutSource::Listed(
            [
                vec!["k1".to_string()],
                vec!["k1".into(), "k2".into(), "k3".into()],
                vec!["a".into(), "b".into()],
            ][r.below(3) as usize]
                .clone(),
        )
    };
    Stage::Fanout {
        source,
        join: [FanoutJoin::All, FanoutJoin::First, FanoutJoin::Pick][r.below(3) as usize],
    }
}
fn rand_output(r: &mut Pcg) -> WorkOutput {
    [WorkOutput::Branch, WorkOutput::Result, WorkOutput::Plan][r.below(3) as usize]
}

fn gen_wf(r: &mut Pcg) -> Workflow {
    let mut kinds: Vec<Stage> = Vec::new();
    match r.below(4) {
        0 => {
            for _ in 0..1 + r.below(8) {
                let s = match r.below(12) {
                    0..=3 => {
                        let o = rand_output(r);
                        mk_work(r, o)
                    }
                    4 => Stage::Submit {
                        forge: ["local", "github"][r.below(2) as usize].into(),
                    },
                    5 | 6 => mk_cmd(r),
                    7 | 8 => {
                        let b = r.pct(55);
                        mk_appr(r, b)
                    }
                    9 => Stage::Merge,
                    _ => mk_fanout(r, true),
                };
                kinds.push(s);
            }
        }
        _ => {
            let mut has_plan = false;
            if r.pct(45) {
                let o = if r.pct(50) {
                    WorkOutput::Plan
                } else {
                    WorkOutput::Result
                };
                has_plan = o == WorkOutput::Plan;
                kinds.push(mk_work(r, o));
                match r.below(3) {
                    0 => kinds.push(mk_appr(r, false)),
                    1 => {
                        let f = mk_fanout(r, has_plan);
                        let pick = matches!(
                            f,
                            Stage::Fanout {
                                join: FanoutJoin::Pick,
                                ..
                            }
                        );
                        kinds.push(f);
                        if pick {
                            let b = r.pct(20);
                            kinds.push(mk_appr(r, b));
                        }
                    }
                    _ => {}
                }
            }
            kinds.push(mk_work(r, WorkOutput::Branch));
            if r.pct(20) {
                kinds.push(mk_appr(r, false));
                kinds.push(mk_work(r, WorkOutput::Branch));
            }
            let mut post: Vec<Stage> = Vec::new();
            for _ in 0..r.below(5) {
                match r.below(9) {
                    0 | 1 => post.push(Stage::Submit {
                        forge: ["local", "github"][r.below(2) as usize].into(),
                    }),
                    2 | 3 => post.push(mk_cmd(r)),
                    4 | 5 => {
                        let b = r.pct(70);
                        post.push(mk_appr(r, b))
                    }
                    6 => {
                        let f = mk_fanout(r, has_plan);
                        let pick = matches!(
                            f,
                            Stage::Fanout {
                                join: FanoutJoin::Pick,
                                ..
                            }
                        );
                        post.push(f);
                        if pick {
                            let b = r.pct(50);
                            post.push(mk_appr(r, b));
                        }
                    }
                    7 => {
                        let o = rand_output(r);
                        post.push(mk_work(r, o))
                    }
                    _ => post.push(mk_cmd(r)),
                }
            }
            kinds.extend(post);
            if r.pct(70) {
                kinds.push(Stage::Merge);
            }
            if r.pct(15) && kinds.len() > 2 {
                let a = r.below(kinds.len() as u32) as usize;
                let b = r.below(kinds.len() as u32) as usize;
                kinds.swap(a, b);
            }
            if r.pct(10) && kinds.len() > 2 {
                let a = r.below(kinds.len() as u32) as usize;
                kinds.remove(a);
            }
        }
    }
    let n = kinds.len();
    let mut stages: Vec<WorkflowStage> = kinds
        .into_iter()
        .enumerate()
        .map(|(i, s)| WorkflowStage::new(format!("st{i}"), s))
        .collect();
    for i in 0..n {
        if r.pct(25) {
            let works: Vec<usize> = (0..i)
                .filter(|&j| stages[j].stage.kind() == StageKind::Work)
                .collect();
            let t = if !works.is_empty() && r.pct(85) {
                works[r.below(works.len() as u32) as usize]
            } else {
                r.below(n as u32) as usize
            };
            stages[i].on_fail = Some(format!("st{t}"));
        }
        if r.pct(35) {
            stages[i].timeout_ms = Some([1, 50, 60_000, 0][r.below(4) as usize]);
            stages[i].on_timeout = Some(
                [
                    TimeoutAction::Notify,
                    TimeoutAction::Reassign,
                    TimeoutAction::Cancel,
                ][r.below(3) as usize],
            );
        }
    }
    Workflow {
        id: "a4r3".into(),
        version: 1,
        requires: if r.pct(85) {
            vec![WorkflowRequirement::Repo]
        } else {
            vec![]
        },
        allow_unreviewed: r.pct(30),
        stages,
    }
}

#[derive(Clone)]
struct Node {
    st: PipelineState,
    heads: u8,
    fails: u8,
    ctr: u32,
    patch_of: BTreeMap<String, String>,
    merge_sent: Option<String>,
    cmd_pass: BTreeSet<(String, String)>,
    appr: BTreeSet<(String, Option<String>, Option<String>)>,
}

fn key(n: &Node) -> String {
    format!("{:?}|{}|{}|{:?}", n.st, n.heads, n.fails, n.merge_sent)
}

#[derive(Clone, Copy, PartialEq)]
enum Cls {
    Success,
    Head,
    Fail,
    Noise,
}

fn cur(st: &PipelineState) -> Option<&WorkflowStage> {
    st.current_stage()
}

fn success_events(st: &PipelineState, ctr: &mut u32) -> Vec<PipelineEvent> {
    use PipelineEvent as E;
    if st.status() == PipelineStatus::Pending {
        return vec![E::Start];
    }
    let Some(stage) = cur(st) else { return vec![] };
    let id = stage.id.clone();
    let head = st.current_head().map(str::to_string);
    match &stage.stage {
        Stage::Work { output, .. } => {
            let p = match output {
                WorkOutput::Branch => {
                    *ctr += 1;
                    WorkProduct::Branch {
                        branch: "agend/T/b".into(),
                        head: format!("S{ctr}"),
                        patch_id: format!("SP{ctr}"),
                    }
                }
                WorkOutput::Result => WorkProduct::Result {
                    summary: "s".into(),
                    output: None,
                },
                WorkOutput::Plan => WorkProduct::Plan {
                    items: vec!["i1".into(), "i2".into()],
                },
            };
            vec![E::WorkCompleted { product: p }]
        }
        Stage::Submit { forge } => vec![E::Submitted {
            change_id: (forge != "local").then(|| "42".into()),
        }],
        Stage::Command { .. } => vec![E::CommandFinished {
            stage_id: id,
            head,
            exit_code: Some(0),
        }],
        Stage::Approval { .. } => {
            let reviewer = format!("rv{}", st.approval_reviewers().len());
            let mut choices: Vec<Option<String>> = vec![None];
            choices.extend(st.fanout_child_task_ids().iter().cloned().map(Some));
            choices
                .into_iter()
                .map(|c| E::ApprovalGranted {
                    stage_id: id.clone(),
                    reviewer: reviewer.clone(),
                    head: head.clone(),
                    selected_child: c,
                })
                .collect()
        }
        Stage::Fanout { source, join } => {
            let kids: Vec<String> = match source {
                FanoutSource::Listed(k) => k.clone(),
                FanoutSource::WorkOutput => vec!["p1".into(), "p2".into()],
            };
            if *join == FanoutJoin::First {
                kids.iter()
                    .map(|w| E::FanoutCompleted {
                        stage_id: id.clone(),
                        child_task_ids: kids.clone(),
                        selected_child: Some(w.clone()),
                    })
                    .collect()
            } else {
                vec![E::FanoutCompleted {
                    stage_id: id,
                    child_task_ids: kids,
                    selected_child: None,
                }]
            }
        }
        Stage::Merge => vec![E::MergeCompleted {
            head: head.unwrap_or_default(),
            merge_commit: "M".into(),
        }],
    }
}

fn all_events(n: &Node, ctr: &mut u32) -> Vec<(Cls, PipelineEvent)> {
    use PipelineEvent as E;
    let st = &n.st;
    let mut v: Vec<(Cls, E)> = success_events(st, ctr)
        .into_iter()
        .map(|e| (Cls::Success, e))
        .collect();
    let id = cur(st).map(|s| s.id.clone()).unwrap_or_default();
    let head = st.current_head().map(str::to_string);
    let patch = st.patch_id().map(str::to_string).unwrap_or_default();
    if n.heads > 0 {
        *ctr += 1;
        let c = *ctr;
        v.push((
            Cls::Head,
            E::CommitCreated {
                head: format!("C{c}"),
                patch_id: format!("CP{c}"),
            },
        ));
        v.push((
            Cls::Head,
            E::MainAdvanced {
                rebased_head: format!("K{c}"),
                patch_id: patch.clone(),
                conflict: false,
            },
        ));
        v.push((
            Cls::Head,
            E::MainAdvanced {
                rebased_head: format!("R{c}"),
                patch_id: format!("RP{c}"),
                conflict: false,
            },
        ));
        v.push((
            Cls::Head,
            E::MainAdvanced {
                rebased_head: format!("X{c}"),
                patch_id: format!("XP{c}"),
                conflict: true,
            },
        ));
    }
    if n.fails > 0 {
        v.push((
            Cls::Fail,
            E::CommandFinished {
                stage_id: id.clone(),
                head: head.clone(),
                exit_code: Some(1),
            },
        ));
        v.push((
            Cls::Fail,
            E::CommandFinished {
                stage_id: id.clone(),
                head: head.clone(),
                exit_code: None,
            },
        ));
        v.push((
            Cls::Fail,
            E::ChangesRequested {
                stage_id: id.clone(),
                reviewer: "rvX".into(),
                head: head.clone(),
                reason: "fix".into(),
            },
        ));
        v.push((
            Cls::Fail,
            E::StageFailed {
                stage_id: id.clone(),
                reason: "boom".into(),
            },
        ));
        v.push((
            Cls::Fail,
            E::MergeFailed {
                head: head.clone().unwrap_or_default(),
                reason: "refused".into(),
            },
        ));
    }
    // noise: rework without new commit, stale/duplicate, timeouts
    if let Some(h) = &head {
        v.push((
            Cls::Noise,
            E::WorkCompleted {
                product: WorkProduct::Branch {
                    branch: "agend/T/b".into(),
                    head: h.clone(),
                    patch_id: patch.clone(),
                },
            },
        ));
    }
    v.push((
        Cls::Noise,
        E::WorkCompleted {
            product: WorkProduct::Result {
                summary: "wrong".into(),
                output: None,
            },
        },
    ));
    v.push((
        Cls::Noise,
        E::CommandFinished {
            stage_id: id.clone(),
            head: Some("STALE".into()),
            exit_code: Some(0),
        },
    ));
    v.push((
        Cls::Noise,
        E::CommandFinished {
            stage_id: "nope".into(),
            head: head.clone(),
            exit_code: Some(0),
        },
    ));
    v.push((
        Cls::Noise,
        E::ApprovalGranted {
            stage_id: id.clone(),
            reviewer: "rv0".into(),
            head: Some("STALE".into()),
            selected_child: None,
        },
    ));
    v.push((
        Cls::Noise,
        E::ApprovalGranted {
            stage_id: id.clone(),
            reviewer: "rv0".into(),
            head: head.clone(),
            selected_child: None,
        },
    ));
    v.push((
        Cls::Noise,
        E::ApprovalGranted {
            stage_id: id.clone(),
            reviewer: "rvZ".into(),
            head: head.clone(),
            selected_child: Some("zzz".into()),
        },
    ));
    v.push((
        Cls::Noise,
        E::ApprovalGranted {
            stage_id: "nope".into(),
            reviewer: "rvY".into(),
            head: head.clone(),
            selected_child: None,
        },
    ));
    v.push((
        Cls::Noise,
        E::MergeCompleted {
            head: "STALE".into(),
            merge_commit: "M".into(),
        },
    ));
    v.push((
        Cls::Noise,
        E::MergeFailed {
            head: "STALE".into(),
            reason: "x".into(),
        },
    ));
    v.push((
        Cls::Noise,
        E::FanoutCompleted {
            stage_id: id.clone(),
            child_task_ids: vec![],
            selected_child: None,
        },
    ));
    let local =
        matches!(cur(st).map(|s| &s.stage), Some(Stage::Submit { forge }) if forge == "local");
    v.push((
        Cls::Noise,
        E::Submitted {
            change_id: if local { None } else { Some("43".into()) },
        },
    ));
    if st.status() != PipelineStatus::Pending {
        v.push((Cls::Noise, E::Start));
    }
    v.push((
        Cls::Noise,
        E::StageTimedOut {
            stage_id: id.clone(),
        },
    ));
    v.push((
        Cls::Noise,
        E::Cancel {
            reason: "op".into(),
        },
    ));
    if let Some(h) = &head {
        v.push((
            Cls::Noise,
            E::CommitCreated {
                head: h.clone(),
                patch_id: patch.clone(),
            },
        ));
    }
    v
}

#[derive(Default, Debug)]
struct Stats {
    generated: u64,
    accepted: u64,
    explored_wf: u64,
    states: u64,
    transitions: u64,
    dead_hard: u64,
    dead_soft: u64,
    violations: u64,
    panics: u64,
    truncated: u64,
    done_reached: u64,
    merged: u64,
}

fn violation(stats: &mut Stats, wf: &Workflow, msg: String, shown: &mut u32) {
    stats.violations += 1;
    if *shown < 12 {
        *shown += 1;
        eprintln!(
            "VIOLATION: {msg}\n  workflow: {:?}",
            wf.stages
                .iter()
                .map(|s| (
                    s.id.clone(),
                    format!("{:?}", s.stage),
                    s.on_fail.clone(),
                    s.on_timeout
                ))
                .collect::<Vec<_>>()
        );
    }
}

fn gate_ok(n: &Node) -> Result<(), String> {
    let st = &n.st;
    let wf = st.workflow();
    let head = st.current_head().ok_or("no head")?.to_string();
    let hp = n.patch_of.get(&head).cloned();
    let mi = wf
        .stages
        .iter()
        .position(|s| s.stage.kind() == StageKind::Merge)
        .unwrap();
    for s in &wf.stages[..mi] {
        match &s.stage {
            Stage::Command { .. } => {
                if !n.cmd_pass.contains(&(s.id.clone(), head.clone())) {
                    return Err(format!("command {} never passed at {head}", s.id));
                }
            }
            Stage::Approval {
                bind_head: true, ..
            } => {
                if !n.appr.iter().any(|(id, h, p)| {
                    *id == s.id && (h.as_deref() == Some(&head) || (p.is_some() && *p == hp))
                }) {
                    return Err(format!("bound approval {} not for {head}", s.id));
                }
            }
            Stage::Approval { .. } => {
                if !n.appr.iter().any(|(id, _, _)| *id == s.id) {
                    return Err(format!("approval {} never granted", s.id));
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn can_finish(
    st: &PipelineState,
    depth: u32,
    memo: &mut HashMap<String, bool>,
    ctr: &mut u32,
) -> bool {
    if st.status() == PipelineStatus::Done {
        return true;
    }
    if st.status().is_terminal() || depth == 0 {
        return false;
    }
    let k = format!("{st:?}");
    if let Some(&b) = memo.get(&k) {
        return b;
    }
    let mut ok = false;
    for e in success_events(st, ctr) {
        if let Ok(Ok((nx, _))) = catch_unwind(AssertUnwindSafe(|| step(st, e.clone()))) {
            if nx != *st && can_finish(&nx, depth - 1, memo, ctr) {
                ok = true;
                break;
            }
        }
    }
    if ok {
        memo.insert(k, ok);
    }
    ok
}

fn explore(wf: Workflow, stats: &mut Stats, shown: &mut u32, cap: usize, heads: u8, fails: u8) {
    let vwf = wf.clone().validated(&roles()).expect("accepted");
    stats.explored_wf += 1;
    let has_merge = wf.stages.iter().any(|s| s.stage.kind() == StageKind::Merge);
    let root = Node {
        st: PipelineState::new("T", vwf),
        heads,
        fails,
        ctr: 0,
        patch_of: BTreeMap::new(),
        merge_sent: None,
        cmd_pass: BTreeSet::new(),
        appr: BTreeSet::new(),
    };
    let mut seen: HashSet<String> = HashSet::new();
    let mut q = VecDeque::new();
    seen.insert(key(&root));
    q.push_back(root);
    let mut memo = HashMap::new();
    let depth = (wf.stages.len() as u32) * 6 + 10;
    let mut count = 0usize;
    while let Some(n) = q.pop_front() {
        count += 1;
        stats.states += 1;
        if count > cap {
            stats.truncated += 1;
            break;
        }
        let st = &n.st;
        if st.status() == PipelineStatus::Done {
            stats.done_reached += 1;
        }
        if !st.status().is_terminal() {
            let mut c2 = 1_000_000;
            if !can_finish(st, depth, &mut memo, &mut c2) {
                // soft? try allowing one ChangesRequested / command fail first
                stats.dead_hard += 1;
                violation(
                    stats,
                    &wf,
                    format!(
                        "DEAD END (no success continuation): stage={:?} status={:?} head={:?} children={:?} sel={:?} approvals={:?} checks={:?} pending={:?}",
                        cur(st).map(|s| &s.id),
                        st.status(),
                        st.current_head(),
                        st.fanout_child_task_ids(),
                        st.selected_fanout_child(),
                        st.approvals(),
                        st.passed_checks(),
                        st.pending_head_changes()
                    ),
                    shown,
                );
            }
        }
        let mut ctr = n.ctr;
        for (cls, ev) in all_events(&n, &mut ctr) {
            stats.transitions += 1;
            let r = catch_unwind(AssertUnwindSafe(|| step(st, ev.clone())));
            let Ok(r) = r else {
                stats.panics += 1;
                violation(stats, &wf, format!("PANIC on {ev:?}"), shown);
                continue;
            };
            if st.status().is_terminal() {
                if r.is_ok() {
                    violation(
                        stats,
                        &wf,
                        format!("terminal {:?} accepted {ev:?}", st.status()),
                        shown,
                    );
                }
                continue;
            }
            let Ok((nx, actions)) = r else { continue };
            let mut m = n.clone();
            m.ctr = ctr;
            m.st = nx.clone();
            match cls {
                Cls::Head => m.heads -= 1,
                Cls::Fail => m.fails -= 1,
                _ => {}
            }
            // oracle bookkeeping
            if let PipelineEvent::WorkCompleted {
                product: WorkProduct::Branch { head, patch_id, .. },
            } = &ev
            {
                m.patch_of.insert(head.clone(), patch_id.clone());
            }
            if let PipelineEvent::CommitCreated { head, patch_id } = &ev {
                m.patch_of.entry(head.clone()).or_insert(patch_id.clone());
            }
            if let PipelineEvent::MainAdvanced {
                rebased_head,
                patch_id,
                conflict: false,
            } = &ev
            {
                m.patch_of
                    .entry(rebased_head.clone())
                    .or_insert(patch_id.clone());
            }
            let in_flight = st.merge_in_flight();
            if let PipelineEvent::CommandFinished {
                stage_id,
                head: Some(h),
                exit_code: Some(0),
            } = &ev
            {
                m.cmd_pass.insert((stage_id.clone(), h.clone()));
            }
            if let PipelineEvent::ApprovalGranted { stage_id, head, .. } = &ev {
                if nx.stage_index() != st.stage_index() || nx.status() != st.status() {
                    let bind = matches!(
                        cur(st).map(|s| &s.stage),
                        Some(Stage::Approval {
                            bind_head: true,
                            ..
                        })
                    );
                    let h = if bind { head.clone() } else { None };
                    let p = h.as_ref().and_then(|h| n.patch_of.get(h).cloned());
                    m.appr.insert((stage_id.clone(), h, p));
                }
            }
            // invariants
            if cls == Cls::Noise
                && nx != *st
                && !matches!(
                    ev,
                    PipelineEvent::WorkCompleted { .. }
                        | PipelineEvent::StageTimedOut { .. }
                        | PipelineEvent::Cancel { .. }
                        | PipelineEvent::ApprovalGranted { .. }
                        | PipelineEvent::Submitted { .. }
                )
            {
                violation(
                    stats,
                    &wf,
                    format!(
                        "noise event changed state: {ev:?} at {:?}",
                        cur(st).map(|s| &s.id)
                    ),
                    shown,
                );
            }
            if in_flight
                && matches!(
                    ev,
                    PipelineEvent::CommitCreated { .. } | PipelineEvent::MainAdvanced { .. }
                )
            {
                if nx.stage_index() != st.stage_index()
                    || nx.current_head() != st.current_head()
                    || !actions.is_empty()
                {
                    violation(
                        stats,
                        &wf,
                        format!("head change left in-flight merge: {ev:?}"),
                        shown,
                    );
                }
            }
            if matches!(
                ev,
                PipelineEvent::Cancel { .. } | PipelineEvent::StageTimedOut { .. }
            ) && in_flight
                && nx.status() == PipelineStatus::Cancelled
            {
                violation(stats, &wf, "cancelled in-flight merge".into(), shown);
            }
            for a in &actions {
                match a {
                    PipelineAction::Merge { head, .. } => {
                        m.merge_sent = Some(head.clone());
                        if let Err(e) = gate_ok(&m) {
                            violation(
                                stats,
                                &wf,
                                format!(
                                    "Merge action with gate open by impl but oracle says: {e}; ev={ev:?}"
                                ),
                                shown,
                            );
                        }
                    }
                    PipelineAction::ReturnToWork { stage_id, .. } => {
                        let si = wf.stages.iter().position(|s| s.id == *stage_id).unwrap();
                        if wf.stages[si].stage.kind() != StageKind::Work {
                            violation(
                                stats,
                                &wf,
                                format!("ReturnToWork to non-work {stage_id}"),
                                shown,
                            );
                        }
                        if matches!(
                            ev,
                            PipelineEvent::ChangesRequested { .. }
                                | PipelineEvent::CommandFinished { .. }
                        ) {
                            let fi = st.stage_index();
                            let exp = wf.stages[fi]
                                .on_fail
                                .as_ref()
                                .and_then(|t| {
                                    wf.stages[..fi].iter().position(|s| {
                                        &s.id == t && s.stage.kind() == StageKind::Work
                                    })
                                })
                                .or_else(|| {
                                    wf.stages[..fi]
                                        .iter()
                                        .rposition(|s| s.stage.kind() == StageKind::Work)
                                });
                            if exp != Some(si) {
                                violation(
                                    stats,
                                    &wf,
                                    format!("rework went to {stage_id}, expected {exp:?}"),
                                    shown,
                                );
                            }
                        }
                    }
                    _ => {}
                }
            }
            if nx.status() == PipelineStatus::Done && has_merge {
                stats.merged += 1;
                if let PipelineEvent::MergeCompleted { head, .. } = &ev {
                    if m.merge_sent.as_deref() != Some(head.as_str()) {
                        violation(
                            stats,
                            &wf,
                            format!("merged {head} but sent {:?}", m.merge_sent),
                            shown,
                        );
                    }
                    if let Err(e) = gate_ok(&m) {
                        violation(
                            stats,
                            &wf,
                            format!("Done with oracle gate closed: {e}"),
                            shown,
                        );
                    }
                } else {
                    violation(
                        stats,
                        &wf,
                        format!("Done in merge workflow via {ev:?}"),
                        shown,
                    );
                }
            }
            if nx.status() == PipelineStatus::Running
                && nx.stage_index() > st.stage_index() + 1
                && st.status() == PipelineStatus::Running
            {
                for s in &wf.stages[st.stage_index() + 1..nx.stage_index()] {
                    if s.stage.kind() != StageKind::Approval {
                        violation(
                            stats,
                            &wf,
                            format!("skipped non-approval stage {} on {ev:?}", s.id),
                            shown,
                        );
                    }
                }
            }
            if nx.stage_index() != st.stage_index() || nx.status() != st.status() {
                if !in_flight || !matches!(ev, PipelineEvent::MergeFailed { .. }) {}
                m.merge_sent = if nx.merge_in_flight() {
                    m.merge_sent
                } else {
                    None
                };
            }
            let k = key(&m);
            if seen.insert(k) {
                q.push_back(m);
            }
        }
    }
}

fn builtins() -> Vec<Workflow> {
    vec![
        Workflow::builtin_code(),
        Workflow::builtin_research(),
        Workflow::builtin_epic(),
        Workflow::builtin_planned(),
    ]
}

#[test]
#[ignore = "deep run: verifier r3 dead-end explorer"]
fn a4r3_dead_end_explorer() {
    let total: u64 = std::env::var("A4R3_N")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(60_000);
    let explore_max: u64 = std::env::var("A4R3_EXPLORE")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1500);
    let mut stats = Stats::default();
    let mut shown = 0u32;
    for wf in builtins() {
        explore(wf, &mut stats, &mut shown, 20_000, 3, 3);
    }
    let mut shapes: HashMap<String, u64> = HashMap::new();
    let mut seen_sig: HashSet<String> = HashSet::new();
    let cap: usize = std::env::var("A4R3_CAP")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(6000);
    for i in 0..total {
        let seed = (i ^ 0xA4_3300_0000).wrapping_mul(0x2545F4914F6CDD1D);
        let mut r = Pcg::new(seed, 0xA4 + (i % 7));
        let wf = gen_wf(&mut r);
        stats.generated += 1;
        let ok = catch_unwind(AssertUnwindSafe(|| wf.validate(&roles())));
        let Ok(res) = ok else {
            stats.panics += 1;
            eprintln!("validate panicked {wf:?}");
            continue;
        };
        if res.is_ok() {
            stats.accepted += 1;
            let shape: String = wf
                .stages
                .iter()
                .map(|s| format!("{:?}", s.stage.kind()).chars().next().unwrap())
                .collect();
            *shapes.entry(shape).or_default() += 1;
            let interesting = wf.stages.iter().any(|s| {
                matches!(
                    s.stage,
                    Stage::Merge | Stage::Fanout { .. } | Stage::Approval { count: 2.., .. }
                )
            }) || wf.stages.len() >= 4;
            let sig = format!(
                "{:?}{}",
                wf.stages
                    .iter()
                    .map(|s| (format!("{:?}", s.stage), s.on_fail.clone(), s.on_timeout))
                    .collect::<Vec<_>>(),
                wf.allow_unreviewed
            );
            if interesting && seen_sig.insert(sig) && stats.explored_wf < explore_max + 4 {
                explore(wf, &mut stats, &mut shown, cap, 2, 2);
            }
        }
    }
    eprintln!("A4R3 {stats:?}\ndistinct accepted shapes: {}", shapes.len());
    let mut top: Vec<_> = shapes.into_iter().collect();
    top.sort_by(|a, b| b.1.cmp(&a.1));
    eprintln!("top shapes: {:?}", &top[..top.len().min(15)]);
    assert_eq!(stats.violations, 0, "violations found");
}
