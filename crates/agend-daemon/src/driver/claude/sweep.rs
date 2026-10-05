//! D40 P9: exact Claude session or agend channel instance argv markers.
//! The supervisor establishes holder death; the shared sweep checks the
//! pgid range and current membership before signalling the whole group.
use crate::driver::codex::sweep::{Swept, sweep_matching};
use std::path::Path;

pub struct Markers<'a> {
    pub session: Option<&'a str>,
    pub instance: &'a str,
    pub agend: &'a Path,
}

impl Markers<'_> {
    fn matches(&self, argv: &[String]) -> bool {
        self.session.is_some_and(|session| {
            argv.windows(2)
                .any(|w| matches!(w[0].as_str(), "--session-id" | "--resume") && w[1] == session)
        }) || (argv.first().is_some_and(|arg| Path::new(arg) == self.agend)
            && argv.get(1).is_some_and(|arg| arg == "channel")
            && argv[2..]
                .windows(2)
                .any(|w| w[0] == "--instance" && w[1] == self.instance))
    }
}

pub fn sweep(pgid: u32, markers: &Markers<'_>) -> Swept {
    sweep_matching(pgid, |argv| markers.matches(argv))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::driver::codex::sweep::group_members;
    use std::os::unix::process::{CommandExt, ExitStatusExt};
    use std::process::{Child, Command};
    use std::time::{Duration, Instant};

    fn markers() -> Markers<'static> {
        Markers {
            session: Some("11111111-1111-4111-8111-111111111111"),
            instance: "g12-sweep",
            agend: Path::new("/fixture/agend"),
        }
    }

    #[test]
    fn attribution_requires_whole_session_arguments_or_the_exact_channel_executable() {
        let m = markers();
        let argv = |args: &[&str]| args.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        for flag in ["--session-id", "--resume"] {
            assert!(m.matches(&argv(&["claude", flag, m.session.unwrap()])));
            for wrong in ["--session", "resume", "--session-id=other"] {
                assert!(!m.matches(&argv(&["claude", wrong, m.session.unwrap()])));
            }
            assert!(!m.matches(&argv(&[
                "claude",
                flag,
                &format!("x{}", m.session.unwrap())
            ])));
        }
        assert!(m.matches(&argv(&[
            "/fixture/agend",
            "channel",
            "--instance",
            "g12-sweep"
        ])));
        for bad in [
            vec!["/fixture/other-agend", "channel", "--instance", "g12-sweep"],
            vec!["/fixture/agend", "hook", "--instance", "g12-sweep"],
            vec!["/fixture/agend", "channel", "--instance", "xg12-sweep"],
            vec!["/fixture/agend", "channel", "--instance=g12-sweep"],
            vec!["claude", "g12-sweep"],
            vec!["codex", "resume", m.session.unwrap()],
        ] {
            assert!(!m.matches(&argv(&bad)), "{bad:?}");
        }
        assert!(!m.matches(&[]));
        assert!(!m.matches(&argv(&["/fixture/agend"])));
    }

    struct Group(Child);
    impl Group {
        fn new(args: &[&str]) -> Self {
            Self(
                Command::new("/bin/sh")
                    .args(["-c", "sleep 60; :", "claude-sweep-probe"])
                    .args(args)
                    .process_group(0)
                    .spawn()
                    .unwrap(),
            )
        }
        fn pgid(&self) -> u32 {
            self.0.id()
        }
        fn wait_members(&self) {
            let until = Instant::now() + Duration::from_secs(5);
            while group_members(self.pgid()).len() < 2 && Instant::now() < until {
                std::thread::sleep(Duration::from_millis(20));
            }
            assert!(group_members(self.pgid()).len() >= 2);
        }
    }
    impl Drop for Group {
        fn drop(&mut self) {
            // These groups were created by this test; always reap on panic too.
            if matches!(self.0.try_wait(), Ok(None)) {
                unsafe {
                    libc::killpg(self.pgid() as i32, libc::SIGKILL);
                }
            }
            let _ = self.0.wait();
        }
    }

    #[test]
    fn native_unrelated_group_survives_and_exact_session_group_is_killed() {
        let m = markers();
        let mut unrelated = Group::new(&["--resume", "x11111111-1111-4111-8111-111111111111"]);
        unrelated.wait_members();
        assert!(matches!(sweep(unrelated.pgid(), &m), Swept::NoMatch(_)));
        assert!(unrelated.0.try_wait().unwrap().is_none());

        let mut ours = Group::new(&["--session-id", m.session.unwrap()]);
        ours.wait_members();
        assert!(matches!(sweep(ours.pgid(), &m), Swept::Killed(_)));
        assert_eq!(ours.0.wait().unwrap().signal(), Some(libc::SIGKILL));
    }

    #[test]
    fn invalid_groups_are_never_inspected_or_signalled() {
        for pgid in [0, 1, i32::MAX as u32 + 1, u32::MAX] {
            assert_eq!(sweep(pgid, &markers()), Swept::OutOfRange);
        }
    }
}
