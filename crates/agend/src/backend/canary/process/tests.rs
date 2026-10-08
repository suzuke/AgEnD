use super::*;
use agend_testkit::tempdir::TempDir;
use std::os::unix::fs::symlink;
use std::os::unix::net::UnixListener;
use std::os::unix::process::ExitStatusExt;

fn native_probe(root: &Path) -> PathBuf {
    let source = root.join("probe.c");
    let program = root.join("probe");
    fs::write(
        &source,
        r#"
#include <unistd.h>
#include <sys/wait.h>
#include <spawn.h>
#include <pthread.h>
#include <errno.h>
#include <stdio.h>
extern char **environ;
static void *thread(void *p) { return p; }
int main(void) {
    pthread_t t;
    int th = pthread_create(&t, 0, thread, 0);
    if (!th) pthread_join(t, 0);
    errno = 0;
    pid_t p = fork();
    int e = errno;
    if (p == 0) {
        setsid();
        FILE *f = fopen("escaped-marker", "w");
        if (f) { fputs("escaped", f); fclose(f); }
        _exit(0);
    }
    if (p > 0) waitpid(p, 0, 0);
    char *args[] = {"/usr/bin/true", 0};
    pid_t spawned = 0;
    int sp = posix_spawn(&spawned, args[0], 0, 0, args, environ);
    if (!sp) waitpid(spawned, 0, 0);
    if (th || p != -1 || e != EPERM || sp != EPERM) return 2;
    printf("native-probe\n");
    return 0;
}
"#,
    )
    .unwrap();
    assert!(
        Command::new("/usr/bin/cc")
            .arg(source)
            .arg("-pthread")
            .arg("-o")
            .arg(&program)
            .status()
            .unwrap()
            .success()
    );
    program
}

#[test]
fn version_probe_blocks_fork_and_spawn_but_preserves_threads() {
    let root = TempDir::new("g13-probe-child").unwrap();
    let program = native_probe(root.path());
    let mut lab = Lab::new(&crate::backend::canary::uuid().unwrap()).unwrap();
    assert_eq!(
        lab.version(&program, Instant::now() + Duration::from_secs(5))
            .unwrap(),
        "native-probe\n"
    );
    assert!(lab.probe.is_none());
    assert!(!lab.home.join("escaped-marker").exists());
    lab.cleanup().unwrap();
    assert!(!lab.home.exists());
}

#[test]
fn cleanup_refuses_redirected_run_directory_before_connecting() {
    let mut foreign = Lab::new(&crate::backend::canary::uuid().unwrap()).unwrap();
    fs::create_dir(foreign.home.as_path().join("holders")).unwrap();
    let socket = foreign.home.as_path().join("holders/canary.sock");
    let listener = UnixListener::bind(socket).unwrap();
    listener.set_nonblocking(true).unwrap();
    fs::write(foreign.home.as_path().join("sentinel"), b"untouched").unwrap();
    let mut lab = Lab::new(&crate::backend::canary::uuid().unwrap()).unwrap();
    symlink(foreign.home.as_path(), lab.home.join("run")).unwrap();
    assert!(lab.cleanup().is_err());
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    assert_eq!(
        fs::read(foreign.home.as_path().join("sentinel")).unwrap(),
        b"untouched"
    );
    fs::remove_file(lab.home.join("run")).unwrap();
    lab.check_home().unwrap();
    fs::remove_dir_all(&lab.home).unwrap();
    drop(listener);
    foreign.cleanup().unwrap();
}

#[test]
fn version_timeout_still_cleans_the_owned_probe() {
    let mut lab = Lab::new(&crate::backend::canary::uuid().unwrap()).unwrap();
    // sleep rejects --version promptly on macOS; use a native waiting producer.
    let root = TempDir::new("g13-probe-timeout").unwrap();
    let source = root.path().join("wait.c");
    let program = root.path().join("wait");
    fs::write(
        &source,
        "#include <unistd.h>\nint main(void) { sleep(20); return 0; }\n",
    )
    .unwrap();
    assert!(
        Command::new("/usr/bin/cc")
            .arg(source)
            .arg("-pthread")
            .arg("-o")
            .arg(&program)
            .status()
            .unwrap()
            .success()
    );
    assert!(
        lab.version(&program, Instant::now() + Duration::from_millis(100))
            .unwrap_err()
            .contains("timed out")
    );
    lab.cleanup().unwrap();
    assert!(!lab.home.exists());
}

#[test]
fn a_probe_that_moves_itself_to_a_new_session_is_still_reaped() {
    let mut lab = Lab::new(&crate::backend::canary::uuid().unwrap()).unwrap();
    let root = TempDir::new("g13-probe-session").unwrap();
    let source = root.path().join("session.c");
    let program = root.path().join("session");
    fs::write(
        &source,
        r#"
#include <unistd.h>
#include <stdio.h>
#include <errno.h>
int main(void) {
    FILE *d = fopen("session-progress", "w");
    if (!d) return 4;
    fprintf(d, "pid=%d parent=%d group=%d parent_group=%d\n",
        getpid(), getppid(), getpgid(0), getpgid(getppid()));
    fflush(d);
    if (setpgid(0, getpgid(getppid()))) {
        fprintf(d, "setpgid errno=%d\n", errno); fclose(d); return 2;
    }
    fprintf(d, "moved group=%d session=%d\n", getpgid(0), getsid(0));
    fflush(d);
    /* Session preparation may briefly see the old group still registered.
       Only retry EPERM while we remain in the parent's group. This fixture
       must actually establish a new session before testing production cleanup. */
    int attempts;
    for (attempts = 0; attempts < 100; ++attempts) {
        if (setsid() >= 0) break;
        int saved = errno;
        if (attempts == 0)
            fprintf(d, "setsid errno=%d group=%d session=%d\n",
                saved, getpgid(0), getsid(0));
        if (saved != EPERM || getpgid(0) == getpid() ||
            getpgid(0) != getpgid(getppid())) {
            fclose(d); return 2;
        }
        usleep(10000);
    }
    if (attempts == 100 || getsid(0) != getpid() || getpgid(0) != getpid()) {
        fprintf(d, "session preparation failed after %d attempts\n", attempts);
        fclose(d); return 2;
    }
    fprintf(d, "detached attempts=%d group=%d session=%d\n",
        attempts + 1, getpgid(0), getsid(0)); fclose(d);
    FILE *f = fopen("detached-self", "w");
    if (!f) return 3;
    fputs("ready", f); fclose(f);
    sleep(20);
    return 0;
}
"#,
    )
    .unwrap();
    assert!(
        Command::new("/usr/bin/cc")
            .arg(source)
            .arg("-o")
            .arg(&program)
            .status()
            .unwrap()
            .success()
    );
    // The production deadline is unchanged; on failure distinguish startup,
    // group movement and cleanup without logging backend output or credentials.
    let error = lab
        .version(&program, Instant::now() + Duration::from_secs(5))
        .unwrap_err();
    assert!(
        error.contains("timed out"),
        "{error}; fixture progress: {:?}",
        fs::read_to_string(lab.home.join("session-progress"))
    );
    assert_eq!(fs::read(lab.home.join("detached-self")).unwrap(), b"ready");
    assert!(!exited_unreaped(&lab.probe.as_ref().unwrap().child).unwrap());
    lab.cleanup().unwrap();
    assert_eq!(
        lab.probe.as_ref().unwrap().status.unwrap().signal(),
        Some(libc::SIGKILL)
    );
    assert!(!lab.home.exists());
}
