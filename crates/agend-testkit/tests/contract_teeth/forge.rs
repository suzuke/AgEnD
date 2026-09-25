//! Forge mutants: a `FakeForge` with one method replaced.

use std::collections::BTreeMap;
use std::sync::Mutex;

use agend_core::traits::{Forge, MergeRequest, MergeResult, Submission, SubmittedChange};
use agend_testkit::block_on;
use agend_testkit::contract::forge::{self, ForgeFixture};
use agend_testkit::fakes::{FakeError, FakeForge};

use super::Mutant;

type Submit = fn(&M, &Submission) -> Result<SubmittedChange, FakeError>;
type Head = fn(&M, &str) -> Result<String, FakeError>;
type Merge = fn(&M, &MergeRequest) -> Result<MergeResult, FakeError>;

/// A fake forge with `submit`, `head` or `merge_if_head_is` replaced.
pub struct M {
    forge: FakeForge,
    memo: Mutex<BTreeMap<String, String>>,
    submit: Submit,
    head: Head,
    merge: Merge,
}

impl M {
    fn new() -> Self {
        Self {
            forge: FakeForge::new(),
            memo: Mutex::new(BTreeMap::new()),
            submit: |m, c| m.real_submit(c),
            head: |m, b| m.real_head(b),
            merge: |m, r| m.real_merge(r),
        }
    }

    fn submit(self, submit: Submit) -> Self {
        Self { submit, ..self }
    }

    fn head(self, head: Head) -> Self {
        Self { head, ..self }
    }

    fn merge(self, merge: Merge) -> Self {
        Self { merge, ..self }
    }

    fn real_submit(&self, change: &Submission) -> Result<SubmittedChange, FakeError> {
        block_on(self.forge.submit(change))
    }

    fn real_head(&self, branch: &str) -> Result<String, FakeError> {
        block_on(self.forge.head(branch))
    }

    fn real_merge(&self, request: &MergeRequest) -> Result<MergeResult, FakeError> {
        block_on(self.forge.merge_if_head_is(request))
    }

    /// Merges whatever the branch head is now.
    fn merge_current(&self, branch: &str) -> Result<MergeResult, FakeError> {
        let current = self.real_head(branch)?;
        self.real_merge(&MergeRequest {
            branch: branch.to_owned(),
            expected_head: current,
        })
    }
}

impl Forge for M {
    type Error = FakeError;
    async fn submit(&self, change: &Submission) -> Result<SubmittedChange, FakeError> {
        (self.submit)(self, change)
    }
    async fn head(&self, branch: &str) -> Result<String, FakeError> {
        (self.head)(self, branch)
    }
    async fn merge_if_head_is(&self, request: &MergeRequest) -> Result<MergeResult, FakeError> {
        (self.merge)(self, request)
    }
}

impl ForgeFixture for M {
    type Forge = Self;
    type Error = FakeError;
    fn forge(&self) -> &Self {
        self
    }
    fn commit_to(&self, branch: &str) -> String {
        self.forge.push(branch)
    }
    fn base_head(&self) -> String {
        self.forge.base_head()
    }
}

pub fn mutants() -> Vec<Mutant> {
    vec![
        // FRG-1: the first head seen for a branch is cached forever.
        Mutant {
            rule: "FRG-1",
            name: "CachesFirstHead",
            run: |name| {
                forge::run(name, || {
                    M::new().head(|m, b| {
                        if let Some(head) = m.memo.lock().unwrap().get(b) {
                            return Ok(head.clone());
                        }
                        let head = m.real_head(b)?;
                        m.memo.lock().unwrap().insert(b.to_owned(), head.clone());
                        Ok(head)
                    })
                })
            },
        },
        // FRG-2: an unknown branch reports the base head.
        Mutant {
            rule: "FRG-2",
            name: "UnknownBranchHeadIsBase",
            run: |name| {
                forge::run(name, || {
                    M::new().head(|m, b| m.real_head(b).or_else(|_| Ok(m.forge.base_head())))
                })
            },
        },
        // FRG-3: the submitted change has no id.
        Mutant {
            rule: "FRG-3",
            name: "SubmitWithoutId",
            run: |name| {
                forge::run(name, || {
                    M::new().submit(|m, c| {
                        let mut change = m.real_submit(c)?;
                        change.id.clear();
                        Ok(change)
                    })
                })
            },
        },
        // FRG-4: submitting an unknown branch opens an empty change.
        Mutant {
            rule: "FRG-4",
            name: "SubmitsUnknownBranch",
            run: |name| {
                forge::run(name, || {
                    M::new().submit(|m, c| {
                        m.real_submit(c).or_else(|_| {
                            Ok(SubmittedChange {
                                id: "change-ghost".into(),
                                url: None,
                                head: "0".repeat(40),
                            })
                        })
                    })
                })
            },
        },
        // FRG-5: reports Merged without merging anything.
        Mutant {
            rule: "FRG-5",
            name: "MergedWithoutMerging",
            run: |name| {
                forge::run(name, || {
                    M::new().merge(|m, r| {
                        let current = m.real_head(&r.branch)?;
                        Ok(if current == r.expected_head {
                            MergeResult::Merged {
                                merge_commit: format!("{current}-merged"),
                            }
                        } else {
                            MergeResult::HeadChanged {
                                actual_head: current,
                            }
                        })
                    })
                })
            },
        },
        // FRG-5 (verifier r2 F1): compares with the head recorded at submit.
        Mutant {
            rule: "FRG-5",
            name: "ComparesSubmittedHead",
            run: |name| {
                forge::run(name, || {
                    M::new()
                        .submit(|m, c| {
                            let change = m.real_submit(c)?;
                            m.memo
                                .lock()
                                .unwrap()
                                .insert(c.branch.clone(), change.head.clone());
                            Ok(change)
                        })
                        .merge(|m, r| {
                            let current = m.real_head(&r.branch)?;
                            let submitted = m.memo.lock().unwrap().get(&r.branch).cloned();
                            if submitted.as_deref() == Some(r.expected_head.as_str()) {
                                m.merge_current(&r.branch)
                            } else {
                                Ok(MergeResult::HeadChanged {
                                    actual_head: current,
                                })
                            }
                        })
                })
            },
        },
        // FRG-6 (gate page step 4): the refusal echoes the expected head.
        Mutant {
            rule: "FRG-6",
            name: "EchoesExpectedHead",
            run: |name| {
                forge::run(name, || {
                    M::new().merge(|m, r| match m.real_merge(r)? {
                        MergeResult::HeadChanged { .. } => Ok(MergeResult::HeadChanged {
                            actual_head: r.expected_head.clone(),
                        }),
                        merged => Ok(merged),
                    })
                })
            },
        },
        // FRG-7: merges whatever head it is given.
        Mutant {
            rule: "FRG-7",
            name: "MergesStaleHeads",
            run: |name| forge::run(name, || M::new().merge(|m, r| m.merge_current(&r.branch))),
        },
        // FRG-7 (verifier r1 HIGH): merges, then claims the head changed.
        Mutant {
            rule: "FRG-7",
            name: "MergesThenRefuses",
            run: |name| {
                forge::run(name, || {
                    M::new().merge(|m, r| {
                        let current = m.real_head(&r.branch)?;
                        let merged = m.merge_current(&r.branch)?;
                        Ok(if current == r.expected_head {
                            merged
                        } else {
                            MergeResult::HeadChanged {
                                actual_head: current,
                            }
                        })
                    })
                })
            },
        },
        // FRG-8 (verifier r2 F4): prefix match, so "" or a short SHA merges.
        Mutant {
            rule: "FRG-8",
            name: "PrefixMatchesHead",
            run: |name| {
                forge::run(name, || {
                    M::new().merge(|m, r| {
                        let current = m.real_head(&r.branch)?;
                        if current.starts_with(&r.expected_head) {
                            m.merge_current(&r.branch)
                        } else {
                            Ok(MergeResult::HeadChanged {
                                actual_head: current,
                            })
                        }
                    })
                })
            },
        },
        // FRG-9: an unknown branch is reported as a changed head.
        Mutant {
            rule: "FRG-9",
            name: "UnknownBranchMergeIsHeadChanged",
            run: |name| {
                forge::run(name, || {
                    M::new().merge(|m, r| {
                        m.real_merge(r).or_else(|_| {
                            Ok(MergeResult::HeadChanged {
                                actual_head: String::new(),
                            })
                        })
                    })
                })
            },
        },
    ]
}
