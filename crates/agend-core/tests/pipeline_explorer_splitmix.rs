//! Second, independent event-sequence explorer (written by the fresh-context
//! gate 1 verifier, ported here). A SplitMix64 generator drives sequences
//! through its own workflow shapes; its oracle counts a check or approval only
//! if it happened after the most recent completion of the work stage before
//! it (so rework forgets it) and carries approvals across clean same-patch
//! rebases (D14). It checks the merge gate, no skipped stage, `Submitted`
//! before anything past submit, head changes never moving forward or acting
//! in work, reviews and results only for their own stage and head, rework
//! never failing the task, and terminal states staying terminal.
//! `AGEND_EXPLORER2_SEQUENCES` overrides the per-workflow count.

use agend_core::pipeline::stage::{FanoutJoin, StageKind};
use agend_core::pipeline::state::*;
use agend_core::pipeline::workflow::*;
use std::collections::{BTreeMap, BTreeSet};

struct Sm(u64);
impl Sm {
    fn n(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }
    fn b(&mut self, k: usize) -> usize {
        (self.n() % k as u64) as usize
    }
    fn p(&mut self, pc: usize) -> bool {
        self.b(100) < pc
    }
}

fn roles() -> Vec<String> {
    ["dev", "reviewer", "planner"].map(String::from).to_vec()
}
fn w(id: &str, o: WorkOutput) -> WorkflowStage {
    WorkflowStage::new(
        id,
        Stage::Work {
            role: if o == WorkOutput::Plan {
                "planner"
            } else {
                "dev"
            }
            .into(),
            instructions: String::new(),
            output: o,
        },
    )
}
fn cmd(id: &str) -> WorkflowStage {
    WorkflowStage::new(
        id,
        Stage::Command {
            command: format!("run {id} {{head}}"),
        },
    )
}
fn appr(id: &str, count: u8, bind: bool) -> WorkflowStage {
    WorkflowStage::new(
        id,
        Stage::Approval {
            by: Approver::Role("reviewer".into()),
            count,
            bind_head: bind,
        },
    )
}
fn sub() -> WorkflowStage {
    WorkflowStage::new(
        "submit",
        Stage::Submit {
            forge: "github".into(),
        },
    )
}
fn merge() -> WorkflowStage {
    WorkflowStage::new("merge", Stage::Merge)
}
fn wf(stages: Vec<WorkflowStage>) -> Workflow {
    let mut x = Workflow::builtin_code();
    x.id = "t".into();
    x.stages = stages;
    x
}

fn workflows() -> Vec<(&'static str, Workflow)> {
    use WorkOutput::*;
    let mut c2 = cmd("c2");
    c2.on_fail = Some("work".into());
    let mut b = appr("b", 2, true);
    b.on_fail = Some("work".into());
    b.on_timeout = Some(TimeoutAction::Cancel);
    let mut pickw = wf(vec![
        w("plan", Plan),
        WorkflowStage::new(
            "fan",
            Stage::Fanout {
                source: FanoutSource::WorkOutput,
                join: FanoutJoin::Pick,
            },
        ),
        appr("pick", 1, false),
        w("work", Branch),
        sub(),
        cmd("c1"),
        appr("rv", 1, true),
        merge(),
    ]);
    pickw.id = "pick".into();
    let v = vec![
        (
            "mixed",
            wf(vec![
                w("work", Branch),
                sub(),
                cmd("c1"),
                appr("a", 1, false),
                appr("b", 2, true),
                cmd("c2"),
                merge(),
            ]),
        ),
        (
            "cmd-after-approval-onfail",
            wf(vec![
                w("work", Branch),
                cmd("c1"),
                sub(),
                appr("a", 3, false),
                b,
                c2,
                appr("h", 1, true),
                merge(),
            ]),
        ),
        (
            "approval-first",
            wf(vec![
                w("work", Branch),
                appr("early", 1, true),
                sub(),
                cmd("c1"),
                merge(),
            ]),
        ),
        ("pick-then-code", pickw),
        ("code", Workflow::builtin_code()),
        ("planned", Workflow::builtin_planned()),
    ];
    for (n, x) in &v {
        assert_eq!(x.validate(&roles()), Ok(()), "{n}");
    }
    v
}

#[derive(Default)]
struct Or {
    head: Option<String>,
    patch: BTreeMap<String, String>,
    // equivalence: head -> set of heads its approvals carry to
    carry: BTreeMap<String, BTreeSet<String>>,
    epoch: BTreeMap<usize, usize>, // work stage index -> event counter of last WorkCompleted there
    passes: Vec<(usize, String, Option<String>)>, // (t, stage, head)
    approvals: Vec<(usize, String, String, Option<String>)>, // (t, stage, reviewer, head)
    submitted_at: Option<usize>,
    t: usize,
}
impl Or {
    fn covers(&self, given: &str, cur: &str) -> bool {
        given == cur || self.carry.get(given).is_some_and(|s| s.contains(cur))
    }
    fn prev_work(wf: &Workflow, i: usize) -> Option<usize> {
        wf.stages[..i]
            .iter()
            .rposition(|s| s.stage.kind() == StageKind::Work)
    }
    fn since(&self, wf: &Workflow, i: usize) -> usize {
        Self::prev_work(wf, i)
            .and_then(|w| self.epoch.get(&w).copied())
            .unwrap_or(0)
    }
    fn stage_ok(&self, wf: &Workflow, i: usize, head: Option<&str>) -> bool {
        let st = &wf.stages[i];
        let from = self.since(wf, i);
        match &st.stage {
            Stage::Command { .. } => self.passes.iter().any(|(t, s, h)| {
                *t >= from && *s == st.id && h.as_deref() == head && head.is_some()
            }),
            Stage::Approval {
                count, bind_head, ..
            } => {
                let r: BTreeSet<&String> = self
                    .approvals
                    .iter()
                    .filter(|(t, s, _, h)| {
                        *t >= from
                            && *s == st.id
                            && (!bind_head
                                || matches!((h, head), (Some(g), Some(c)) if self.covers(g, c)))
                    })
                    .map(|x| &x.2)
                    .collect();
                r.len() >= usize::from(*count)
            }
            _ => true,
        }
    }
    fn gate(&self, wf: &Workflow, upto: usize, head: Option<&str>) -> Result<(), String> {
        for i in 0..upto {
            if !self.stage_ok(wf, i, head) {
                return Err(format!(
                    "stage {} not satisfied at {head:?}",
                    wf.stages[i].id
                ));
            }
        }
        Ok(())
    }
}

fn genev(r: &mut Sm, s: &PipelineState, heads: &mut Vec<String>, o: &Or) -> PipelineEvent {
    let ids: Vec<String> = s
        .workflow()
        .stages
        .iter()
        .map(|x| x.id.clone())
        .chain(["zz".into()])
        .collect();
    let anyhead = |r: &mut Sm, heads: &Vec<String>| {
        if heads.is_empty() || r.p(10) {
            None
        } else {
            Some(heads[r.b(heads.len())].clone())
        }
    };
    let cur = s.current_stage().map(|x| x.id.clone()).unwrap_or_default();
    let sid = |r: &mut Sm| {
        if r.p(70) {
            cur.clone()
        } else {
            ids[r.b(ids.len())].clone()
        }
    };
    let hd = |r: &mut Sm, heads: &Vec<String>| {
        if r.p(75) {
            s.current_head().map(String::from)
        } else {
            anyhead(r, heads)
        }
    };
    let rv = |r: &mut Sm| ["x", "y", "z", "q"][r.b(4)].to_string();
    let fresh = |heads: &mut Vec<String>| {
        let h = format!("h{}", heads.len());
        heads.push(h.clone());
        h
    };
    if s.status() == PipelineStatus::Pending && r.p(90) {
        return PipelineEvent::Start;
    }
    if r.p(55)
        && let Some(st) = s.current_stage()
    {
        match &st.stage {
            Stage::Submit { .. } => {
                return PipelineEvent::Submitted {
                    change_id: Some("5".into()),
                };
            }
            Stage::Command { .. } => {
                return PipelineEvent::CommandFinished {
                    stage_id: cur.clone(),
                    head: s.current_head().map(String::from),
                    exit_code: if r.p(90) { Some(0) } else { Some(1) },
                };
            }
            Stage::Approval { .. } if r.p(90) => {
                return PipelineEvent::ApprovalGranted {
                    stage_id: cur.clone(),
                    reviewer: rv(r),
                    head: s.current_head().map(String::from),
                    selected_child: s
                        .selected_fanout_child()
                        .map(String::from)
                        .or_else(|| s.fanout_child_task_ids().first().cloned()),
                };
            }
            Stage::Fanout { .. } => {
                return PipelineEvent::FanoutCompleted {
                    stage_id: cur.clone(),
                    child_task_ids: vec!["k1".into(), "k2".into()],
                    selected_child: None,
                };
            }
            Stage::Merge => {
                return PipelineEvent::MergeCompleted {
                    head: s.current_head().map(String::from).unwrap_or_default(),
                    merge_commit: "m".into(),
                };
            }
            _ => {}
        }
    }
    match r.b(15) {
        0 | 1 => PipelineEvent::WorkCompleted {
            product: match s.current_stage().map(|x| &x.stage) {
                Some(Stage::Work {
                    output: WorkOutput::Plan,
                    ..
                }) => WorkProduct::Plan {
                    items: vec!["i".into()],
                },
                _ if r.p(10) => WorkProduct::Result {
                    summary: "s".into(),
                    output: None,
                },
                _ => {
                    let h = if r.p(20) {
                        s.current_head()
                            .map(String::from)
                            .unwrap_or_else(|| fresh(heads))
                    } else {
                        fresh(heads)
                    };
                    let p = format!("p{}", r.b(4));
                    WorkProduct::Branch {
                        branch: "agend/T/b".into(),
                        head: h,
                        patch_id: p,
                    }
                }
            },
        },
        2 => PipelineEvent::Submitted {
            change_id: r.p(70).then(|| "5".into()),
        },
        3 | 4 => PipelineEvent::CommandFinished {
            stage_id: sid(r),
            head: hd(r, heads),
            exit_code: [Some(0), Some(0), Some(0), Some(2), None][r.b(5)],
        },
        5..=7 => PipelineEvent::ApprovalGranted {
            stage_id: sid(r),
            reviewer: rv(r),
            head: hd(r, heads),
            selected_child: if s.fanout_child_task_ids().is_empty() || r.p(10) {
                r.p(5).then(|| "k9".into())
            } else {
                Some(s.fanout_child_task_ids()[r.b(s.fanout_child_task_ids().len())].clone())
            },
        },
        8 => PipelineEvent::ChangesRequested {
            stage_id: sid(r),
            reviewer: rv(r),
            head: hd(r, heads),
            reason: "why".into(),
        },
        9 => PipelineEvent::CommitCreated {
            head: fresh(heads),
            patch_id: format!("p{}", r.b(4)),
        },
        10 => {
            let same = o.head.as_ref().and_then(|h| o.patch.get(h)).cloned();
            PipelineEvent::MainAdvanced {
                rebased_head: if r.p(10) {
                    s.current_head().map(String::from).unwrap_or_default()
                } else {
                    fresh(heads)
                },
                patch_id: if r.p(60) {
                    same.unwrap_or_default()
                } else {
                    format!("p{}", r.b(4))
                },
                conflict: r.p(15),
            }
        }
        11 => PipelineEvent::FanoutCompleted {
            stage_id: sid(r),
            child_task_ids: if r.p(90) {
                vec!["k1".into(), "k2".into()]
            } else {
                vec![]
            },
            selected_child: None,
        },
        12 => PipelineEvent::MergeCompleted {
            head: hd(r, heads).unwrap_or_default(),
            merge_commit: "m".into(),
        },
        13 => {
            if r.p(50) {
                PipelineEvent::StageTimedOut { stage_id: sid(r) }
            } else {
                PipelineEvent::StageFailed {
                    stage_id: sid(r),
                    reason: "f".into(),
                }
            }
        }
        _ => {
            if r.p(30) {
                PipelineEvent::Cancel { reason: "c".into() }
            } else {
                PipelineEvent::Start
            }
        }
    }
}

#[test]
fn splitmix_explorer_keeps_the_pipeline_invariants() {
    let n: usize = std::env::var("AGEND_EXPLORER2_SEQUENCES")
        .ok()
        .and_then(|x| x.parse().ok())
        .unwrap_or(3000);
    let mut totals = String::new();
    for (wi, (name, wfl)) in workflows().into_iter().enumerate() {
        let (mut merged, mut reworks) = (0, 0);
        for seq in 0..n {
            let mut r = Sm(((wi as u64) << 40) ^ (seq as u64).wrapping_mul(0xA24BAED4963EE407));
            let mut s = PipelineState::new("T", wfl.clone().validated(&roles()).unwrap());
            let mut o = Or::default();
            let mut heads = Vec::new();
            let mut trace = Vec::new();
            for _ in 0..80 {
                let e = genev(&mut r, &s, &mut heads, &o);
                trace.push(format!("{e:?}"));
                let res =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| step(&s, e.clone())));
                let res = res.unwrap_or_else(|_| panic!("{name} seq {seq} panic {trace:#?}"));
                let Ok((nx, acts)) = res else { continue };
                let fail = |m: String| -> ! { panic!("{name} seq {seq}: {m}\ntrace {trace:#?}") };
                if s.status().is_terminal() {
                    fail("terminal accepted".into());
                }
                o.t += 1;
                let (from, to) = (s.stage_index(), nx.stage_index());
                let fk = s.current_stage().map(|x| x.stage.kind());
                // record event
                match &e {
                    PipelineEvent::WorkCompleted { product } => {
                        if let WorkProduct::Branch { head, patch_id, .. } = product {
                            if o.head.as_ref() != Some(head) {
                                o.carry.clear();
                            }
                            o.head = Some(head.clone());
                            o.patch.insert(head.clone(), patch_id.clone());
                        }
                        o.epoch.insert(from, o.t);
                        o.submitted_at = None;
                    }
                    PipelineEvent::Submitted { .. } => o.submitted_at = Some(o.t),
                    PipelineEvent::CommandFinished {
                        stage_id,
                        head,
                        exit_code: Some(0),
                    } => o.passes.push((o.t, stage_id.clone(), head.clone())),
                    PipelineEvent::ApprovalGranted {
                        stage_id,
                        reviewer,
                        head,
                        ..
                    } => o
                        .approvals
                        .push((o.t, stage_id.clone(), reviewer.clone(), head.clone())),
                    PipelineEvent::CommitCreated { head, patch_id } => {
                        if o.head.as_ref() != Some(head) {
                            o.head = Some(head.clone());
                            o.patch.insert(head.clone(), patch_id.clone());
                        }
                    }
                    PipelineEvent::MainAdvanced {
                        rebased_head,
                        patch_id,
                        conflict: false,
                    } if o.head.as_ref() != Some(rebased_head) => {
                        let prev = o.head.clone();
                        if let Some(p) = &prev
                            && o.patch.get(p) == Some(patch_id)
                        {
                            let srcs: Vec<String> = o
                                .carry
                                .iter()
                                .filter(|(k, v)| *k == p || v.contains(p))
                                .map(|(k, _)| k.clone())
                                .chain([p.clone()])
                                .collect();
                            for k in srcs {
                                o.carry.entry(k).or_default().insert(rebased_head.clone());
                            }
                        }
                        o.head = Some(rebased_head.clone());
                        o.patch.insert(rebased_head.clone(), patch_id.clone());
                    }
                    _ => {}
                }
                if nx.current_head() != o.head.as_deref() {
                    fail(format!(
                        "head {:?} vs oracle {:?}",
                        nx.current_head(),
                        o.head
                    ));
                }
                // results only for own stage/head
                if let PipelineEvent::CommandFinished { stage_id, head, .. } = &e
                    && (Some(stage_id) != s.current_stage().map(|x| &x.id)
                        || head.as_deref() != s.current_head())
                {
                    fail("foreign command result".into());
                }
                if let PipelineEvent::ApprovalGranted { stage_id, head, .. }
                | PipelineEvent::ChangesRequested { stage_id, head, .. } = &e
                {
                    if Some(stage_id) != s.current_stage().map(|x| &x.id) {
                        fail("foreign review".into());
                    }
                    if matches!(
                        s.current_stage().unwrap().stage,
                        Stage::Approval {
                            bind_head: true,
                            ..
                        }
                    ) && (head.is_none() || head.as_deref() != s.current_head())
                    {
                        fail("review for other head".into());
                    }
                }
                // merge gate
                let mi = wfl
                    .stages
                    .iter()
                    .position(|x| x.stage.kind() == StageKind::Merge);
                for a in &acts {
                    if let PipelineAction::Merge { head, .. } = a
                        && let Err(m) = o.gate(&wfl, mi.unwrap(), Some(head))
                    {
                        fail(format!("Merge action: {m}"));
                    }
                }
                if nx.status() == PipelineStatus::Done {
                    if let Some(mi) = mi {
                        if !matches!(e, PipelineEvent::MergeCompleted { .. }) {
                            fail("done w/o merge".into());
                        }
                        if let Err(m) = o.gate(&wfl, mi, nx.current_head()) {
                            fail(format!("MergeCompleted: {m}"));
                        }
                        if o.submitted_at.is_none() {
                            fail("merged without Submitted since last work".into());
                        }
                        merged += 1;
                    } else if let Err(m) = o.gate(&wfl, wfl.stages.len(), nx.current_head()) {
                        fail(format!("Done: {m}"));
                    }
                }
                // head change never moves forward; in work: nothing
                let hc = matches!(
                    e,
                    PipelineEvent::CommitCreated { .. } | PipelineEvent::MainAdvanced { .. }
                );
                if hc && (to > from || nx.status() != s.status()) {
                    fail("head change moved forward/changed status".into());
                }
                if hc && fk == Some(StageKind::Work) && (to != from || !acts.is_empty()) {
                    fail("head change in work".into());
                }
                // forward moves: completing event + skipped only satisfied approvals
                if to > from && !hc && nx.status() == PipelineStatus::Running
                    || (to > from
                        && nx.status() == PipelineStatus::Done
                        && !matches!(e, PipelineEvent::MergeCompleted { .. }))
                {
                    let okev = matches!(
                        (fk, &e),
                        (Some(StageKind::Work), PipelineEvent::WorkCompleted { .. })
                            | (Some(StageKind::Submit), PipelineEvent::Submitted { .. })
                            | (
                                Some(StageKind::Command),
                                PipelineEvent::CommandFinished {
                                    exit_code: Some(0),
                                    ..
                                }
                            )
                            | (
                                Some(StageKind::Approval),
                                PipelineEvent::ApprovalGranted { .. }
                            )
                            | (
                                Some(StageKind::Fanout),
                                PipelineEvent::FanoutCompleted { .. }
                            )
                    );
                    if !okev || matches!(e, PipelineEvent::Start) && from != 0 {
                        fail(format!("{e:?} advanced {from}->{to}"));
                    }
                    // the completed stage itself must be satisfied now (approval count)
                    if fk == Some(StageKind::Approval) && !o.stage_ok(&wfl, from, nx.current_head())
                    {
                        fail("approval stage advanced below count".into());
                    }
                    for k in from + 1..to.min(wfl.stages.len()) {
                        if wfl.stages[k].stage.kind() != StageKind::Approval
                            || !o.stage_ok(&wfl, k, nx.current_head())
                        {
                            fail(format!("skipped {}", wfl.stages[k].id));
                        }
                    }
                }
                // submit precedes
                if nx.status() == PipelineStatus::Running
                    && let Some(si) = wfl
                        .stages
                        .iter()
                        .position(|x| x.stage.kind() == StageKind::Submit)
                    && to > si
                    && o.submitted_at.is_none()
                {
                    fail(format!("at {to} past submit without Submitted"));
                }
                // rework
                let rework = matches!(
                    e,
                    PipelineEvent::ChangesRequested { .. }
                        | PipelineEvent::CommandFinished {
                            exit_code: Some(1..) | Some(i32::MIN..=-1) | None,
                            ..
                        }
                );
                if rework {
                    reworks += 1;
                    if nx.status() != PipelineStatus::Running {
                        fail(format!("rework failed/ended task: {acts:?}"));
                    }
                    if to >= from {
                        fail("rework did not go back".into());
                    }
                }
                s = nx;
            }
        }
        totals += &format!("{name}: merged {merged} reworks {reworks}\n");
        assert!(
            merged > 0 && reworks > 0,
            "{name}: exploration never merged or reworked"
        );
    }
    eprintln!("{totals}");
}
