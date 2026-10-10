//! Isolated lifecycle tests use a service-manager state model, not invented wire
//! replies. Native parser/lifecycle tests are separate acceptance evidence.

use std::cell::{Cell, RefCell};
use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::Path;

use agend_core::setup::service::{InstallPhase, Installation, Manager, ServiceSpec};
use agend_testkit::tempdir::TempDir;

use super::lifecycle::Owned;
use super::manager::{ServiceManager, State};
use super::{Plan, files};

struct Model {
    state: RefCell<State>,
    launches: Cell<usize>,
    stops: Cell<usize>,
    fail_start_reply: Cell<bool>,
    fail_reload: Cell<bool>,
}
impl Model {
    fn new() -> Self {
        Self {
            state: RefCell::new(State::Absent),
            launches: Cell::new(0),
            stops: Cell::new(0),
            fail_start_reply: Cell::new(false),
            fail_reload: Cell::new(false),
        }
    }
}
impl ServiceManager for Model {
    fn inspect(&self, _: &Installation) -> Result<State, String> {
        Ok(*self.state.borrow())
    }
    fn start(&self, _: &Installation) -> Result<(), String> {
        if *self.state.borrow() == State::Absent {
            self.launches.set(self.launches.get() + 1);
            *self.state.borrow_mut() = State::Owned { running: true };
        }
        if self.fail_start_reply.replace(false) {
            Err("lost manager reply".into())
        } else {
            Ok(())
        }
    }
    fn stop(&self, _: &Installation) -> Result<(), String> {
        self.stops.set(self.stops.get() + 1);
        *self.state.borrow_mut() = State::Absent;
        Ok(())
    }
    fn reload(&self, _: &Installation) -> Result<(), String> {
        if self.fail_reload.replace(false) {
            Err("reload unavailable".into())
        } else {
            Ok(())
        }
    }
}

fn fixture() -> (TempDir, Plan) {
    let root = TempDir::new("g13-lifecycle").unwrap();
    let root_path = root.path().canonicalize().unwrap();
    let home = root_path.join("data");
    let source = root_path.join("source");
    fs::write(&source, b"isolated executable fixture").unwrap();
    fs::set_permissions(&source, fs::Permissions::from_mode(0o700)).unwrap();
    let plan = Plan {
        spec: ServiceSpec {
            manager: Manager::Launchd,
            program: home.join("service/agend").to_string_lossy().into_owned(),
            home: home.to_string_lossy().into_owned(),
            user_home: root_path.to_string_lossy().into_owned(),
            search_path: "/usr/bin:/bin".into(),
        },
        service_path: root_path.join("Library/LaunchAgents/dev.agend.daemon.plist"),
        source,
    };
    (root, plan)
}

#[test]
fn diagnostic_observes_recovery_without_mutating_the_installation_or_manager() {
    use agend_core::setup::CheckStatus;
    let (_root, plan) = fixture();
    let model = Model::new();
    let mut owned = Owned::prepare(&plan, &plan.source, None).unwrap();
    let record_path = Path::new(&plan.spec.home).join("service/installation.json");
    let prepared = fs::read(&record_path).unwrap();
    // Keep the exclusive lifecycle lock held: diagnosis must not create/acquire it.
    let observe = || super::diagnostic::observe(&plan, &model);
    assert_eq!(observe().unwrap().status, CheckStatus::Warn);
    assert_eq!(fs::read(&record_path).unwrap(), prepared);
    assert_eq!(model.launches.get(), 0);
    assert_eq!(model.stops.get(), 0);
    owned.register(&model).unwrap();
    let registered = fs::read(&record_path).unwrap();
    assert_eq!(observe().unwrap().status, CheckStatus::Ok);
    *model.state.borrow_mut() = State::Absent;
    assert_eq!(observe().unwrap().status, CheckStatus::Warn);
    *model.state.borrow_mut() = State::Owned { running: true };
    assert_eq!(observe().unwrap().status, CheckStatus::Ok);
    let definition = fs::read(&plan.service_path).unwrap();
    fs::remove_file(&plan.service_path).unwrap();
    assert!(observe().unwrap_err().contains("missing"));
    assert!(!plan.service_path.exists());
    fs::write(&plan.service_path, &definition).unwrap();
    assert_eq!(observe().unwrap().status, CheckStatus::Ok);
    fs::write(&plan.service_path, b"foreign edit").unwrap();
    assert!(observe().unwrap_err().contains("changed"));
    assert_eq!(fs::read(&plan.service_path).unwrap(), b"foreign edit");
    assert_eq!(fs::read(&record_path).unwrap(), registered);
    assert_eq!(model.launches.get(), 1);
    assert_eq!(model.stops.get(), 0);
}

#[test]
fn round_trip_removes_only_owned_artifacts_and_retains_data() {
    let (_root, plan) = fixture();
    let manager = Model::new();
    let mut owned = Owned::prepare(&plan, &plan.source, Some(&manager)).unwrap();
    let home = std::path::Path::new(&plan.spec.home);
    fs::write(home.join("config.toml"), b"# keep this\n").unwrap();
    files::private_dir(&home.join("bin")).unwrap();
    symlink(&plan.spec.program, home.join("bin/git")).unwrap();
    fs::write(home.join("bin/user-file"), b"keep").unwrap();
    owned.register(&manager).unwrap();
    assert_eq!(owned.record.phase, InstallPhase::Registered);
    drop(owned);
    let mut owned = Owned::prepare(&plan, &plan.source, Some(&manager)).unwrap();
    owned.register(&manager).unwrap();
    assert_eq!(manager.launches.get(), 1);
    symlink("/foreign/tool", home.join("bin/gh")).unwrap();
    owned.remove(&manager).unwrap();
    drop(owned);
    assert!(!plan.service_path.exists());
    assert!(!std::path::Path::new(&plan.spec.program).exists());
    assert!(fs::symlink_metadata(home.join("bin/git")).is_err());
    assert_eq!(
        fs::read_link(home.join("bin/gh")).unwrap(),
        std::path::Path::new("/foreign/tool")
    );
    assert_eq!(fs::read(home.join("bin/user-file")).unwrap(), b"keep");
    assert_eq!(
        fs::read(home.join("config.toml")).unwrap(),
        b"# keep this\n"
    );
    assert!(Owned::open(&plan).unwrap().is_none());
}

#[test]
fn a_lost_registration_reply_is_reconciled_without_a_second_launch() {
    let (_root, plan) = fixture();
    let manager = Model::new();
    manager.fail_start_reply.set(true);
    let mut owned = Owned::prepare(&plan, &plan.source, Some(&manager)).unwrap();
    assert!(owned.register(&manager).is_err());
    assert_eq!(owned.record.phase, InstallPhase::Prepared);
    drop(owned);
    let mut recovered = Owned::open(&plan).unwrap().unwrap();
    recovered.register(&manager).unwrap();
    assert_eq!(manager.launches.get(), 1);
    assert_eq!(recovered.record.phase, InstallPhase::Registered);
}

#[test]
fn partial_removal_retries_from_the_receipt_without_losing_data() {
    let (_root, plan) = fixture();
    let manager = Model::new();
    let mut owned = Owned::prepare(&plan, &plan.source, Some(&manager)).unwrap();
    owned.register(&manager).unwrap();
    manager.fail_reload.set(true);
    assert!(owned.remove(&manager).is_err());
    assert!(!plan.service_path.exists());
    assert!(std::path::Path::new(&plan.spec.program).exists());
    drop(owned);
    assert!(Owned::prepare(&plan, &plan.source, Some(&manager)).is_err());
    let mut recovered = Owned::open(&plan).unwrap().unwrap();
    assert_eq!(recovered.record.phase, InstallPhase::Removing);
    recovered.remove(&manager).unwrap();
    assert!(!std::path::Path::new(&plan.spec.program).exists());
}

#[test]
fn existing_foreign_files_are_not_adopted_even_when_the_bytes_match() {
    let (_root, plan) = fixture();
    fs::create_dir_all(plan.service_path.parent().unwrap()).unwrap();
    let bytes = plan.spec.render().unwrap();
    fs::write(&plan.service_path, &bytes).unwrap();
    assert!(Owned::prepare(&plan, &plan.source, None).is_err());
    assert_eq!(fs::read_to_string(&plan.service_path).unwrap(), bytes);
    assert!(
        files::load(&std::path::Path::new(&plan.spec.home).join("service"))
            .unwrap()
            .is_none()
    );
}

#[test]
fn edited_artifacts_and_redirected_receipts_are_preserved_before_stop() {
    let (_root, plan) = fixture();
    let manager = Model::new();
    let mut owned = Owned::prepare(&plan, &plan.source, None).unwrap();
    owned.register(&manager).unwrap();
    fs::write(&plan.service_path, b"foreign replacement").unwrap();
    assert!(owned.remove(&manager).is_err());
    assert_eq!(manager.stops.get(), 0);
    assert_eq!(
        fs::read(&plan.service_path).unwrap(),
        b"foreign replacement"
    );
    fs::write(&plan.service_path, owned.record.spec.render().unwrap()).unwrap();
    owned.record.service_path = plan.source.to_string_lossy().into_owned();
    let dir = std::path::Path::new(&plan.spec.home).join("service");
    files::save(&dir, &owned.record, true).unwrap();
    drop(owned);
    assert!(Owned::open(&plan).is_err());
    assert_eq!(
        fs::read(&plan.source).unwrap(),
        b"isolated executable fixture"
    );
}

#[test]
fn lock_and_symlink_boundaries_fail_without_following_foreign_targets() {
    let (_root, plan) = fixture();
    let owned = Owned::prepare(&plan, &plan.source, None).unwrap();
    assert!(Owned::open(&plan).is_err());
    drop(owned);
    fs::remove_file(&plan.service_path).unwrap();
    symlink(&plan.source, &plan.service_path).unwrap();
    assert!(Owned::open(&plan).is_err());
    assert_eq!(
        fs::read(&plan.source).unwrap(),
        b"isolated executable fixture"
    );
    fs::remove_file(&plan.service_path).unwrap();
    symlink(plan.source.with_extension("missing"), &plan.service_path).unwrap();
    assert!(Owned::open(&plan).is_err());
}

#[test]
fn redirected_holder_directories_are_refused_before_service_stop() {
    for component in ["run", "run/holders"] {
        let (root, plan) = fixture();
        let manager = Model::new();
        let mut owned = Owned::prepare(&plan, &plan.source, None).unwrap();
        let home = std::path::Path::new(&plan.spec.home);
        let foreign = root.path().join("foreign");
        files::private_dir(&foreign).unwrap();
        fs::write(foreign.join("sentinel"), b"keep").unwrap();
        if component == "run/holders" {
            files::private_dir(&home.join("run")).unwrap();
        }
        symlink(&foreign, home.join(component)).unwrap();
        assert!(owned.remove(&manager).is_err());
        assert_eq!(manager.stops.get(), 0);
        assert_eq!(owned.record.phase, InstallPhase::Prepared);
        assert_eq!(fs::read(foreign.join("sentinel")).unwrap(), b"keep");
        assert!(plan.service_path.exists());
    }
}

#[test]
fn redirected_live_holder_socket_is_refused_without_connecting() {
    use std::io::Write;
    use std::os::fd::AsRawFd;
    use std::os::unix::net::UnixListener;
    let (root, plan) = fixture();
    let manager = Model::new();
    let mut owned = Owned::prepare(&plan, &plan.source, None).unwrap();
    let dir = std::path::Path::new(&plan.spec.home).join("run/holders");
    files::private_dir(&dir).unwrap();
    let mut lock = fs::File::create(dir.join("foreign.lock")).unwrap();
    writeln!(lock, "{}", std::process::id()).unwrap();
    // SAFETY: this test owns the fd and releases the flock by dropping it.
    assert_eq!(
        unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
        0
    );
    let socket = root.path().join("foreign.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    listener.set_nonblocking(true).unwrap();
    symlink(&socket, dir.join("foreign.sock")).unwrap();
    assert!(owned.remove(&manager).is_err());
    assert_eq!(manager.stops.get(), 0);
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    assert!(std::path::Path::new(&plan.spec.program).exists());
}

#[test]
fn a_store_owner_before_socket_bind_blocks_register_and_resource_removal() {
    let (_root, plan) = fixture();
    let manager = Model::new();
    let mut owned = Owned::prepare(&plan, &plan.source, None).unwrap();
    let home = std::path::Path::new(&plan.spec.home);
    let store = agend_daemon::store::SqliteStore::open(home, 0).unwrap();
    assert!(!home.join("run/daemon.sock").exists());
    assert!(owned.register(&manager).is_err());
    assert_eq!(manager.launches.get(), 0);
    assert!(owned.remove(&manager).is_err());
    assert!(plan.service_path.exists());
    assert!(std::path::Path::new(&plan.spec.program).exists());
    drop(store);
    owned.remove(&manager).unwrap();
    assert!(!plan.service_path.exists());
    assert!(home.join("agend.db").exists());
}

#[test]
fn registration_cannot_overwrite_foreign_shims_or_boot_temporaries() {
    for name in ["git", ".git.new"] {
        let (_root, plan) = fixture();
        let manager = Model::new();
        let mut owned = Owned::prepare(&plan, &plan.source, None).unwrap();
        let bin = std::path::Path::new(&plan.spec.home).join("bin");
        files::private_dir(&bin).unwrap();
        let foreign = bin.join(name);
        fs::write(&foreign, b"foreign data").unwrap();
        assert!(owned.register(&manager).is_err());
        assert_eq!(manager.launches.get(), 0);
        assert_eq!(fs::read(&foreign).unwrap(), b"foreign data");
    }
}

#[test]
fn confirmed_data_deletion_keeps_lock_inodes_and_never_follows_links() {
    use std::os::unix::fs::MetadataExt;
    let (root, plan) = fixture();
    let manager = Model::new();
    let mut owned = Owned::prepare(&plan, &plan.source, None).unwrap();
    let home = std::path::Path::new(&plan.spec.home);
    drop(agend_daemon::store::SqliteStore::open(home, 0).unwrap());
    fs::write(home.join("config.toml"), b"private configuration").unwrap();
    files::private_dir(&home.join("archive")).unwrap();
    fs::write(home.join("archive/wip.patch"), b"private patch").unwrap();
    let foreign = root.path().join("foreign-data");
    fs::create_dir(&foreign).unwrap();
    fs::write(foreign.join("sentinel"), b"keep").unwrap();
    symlink(&foreign, home.join("linked-data")).unwrap();
    let locks = [
        home.join(".agend-maintenance.lock"),
        home.join("service/install.lock"),
    ];
    let inodes = locks
        .each_ref()
        .map(|path| fs::metadata(path).unwrap().ino());
    owned.remove_with_data(&manager, true).unwrap();
    assert!(!home.join("agend.db").exists());
    assert!(!home.join("config.toml").exists());
    assert!(!home.join("archive").exists());
    assert!(fs::symlink_metadata(home.join("linked-data")).is_err());
    assert_eq!(fs::read(foreign.join("sentinel")).unwrap(), b"keep");
    assert_eq!(
        locks
            .each_ref()
            .map(|path| fs::metadata(path).unwrap().ino()),
        inodes
    );
    assert_eq!(fs::read_dir(home).unwrap().count(), 2);
    assert_eq!(fs::read_dir(home.join("service")).unwrap().count(), 1);
    drop(owned);
    // The remaining lock scaffolding supports a later clean initialization.
    drop(agend_daemon::store::SqliteStore::open(home, 0).unwrap());
}

#[test]
fn data_deletion_refuses_a_real_git_repository_before_stopping_service() {
    let (_root, plan) = fixture();
    let manager = Model::new();
    let mut owned = Owned::prepare(&plan, &plan.source, None).unwrap();
    let repo = std::path::Path::new(&plan.spec.home).join("workspace/local-repo");
    assert!(
        std::process::Command::new("git")
            .args(["init", "--quiet"])
            .arg(&repo)
            .status()
            .unwrap()
            .success()
    );
    let result = owned.remove_with_data(&manager, true).unwrap_err();
    assert!(result.contains("Git workspace"), "{result}");
    assert_eq!(manager.stops.get(), 0);
    assert!(repo.join(".git/HEAD").is_file());
    assert!(plan.service_path.is_file());
    assert_eq!(owned.record.phase, InstallPhase::Prepared);
}

#[test]
fn data_deletion_refuses_a_live_store_without_removing_data() {
    let (_root, plan) = fixture();
    let manager = Model::new();
    let mut owned = Owned::prepare(&plan, &plan.source, None).unwrap();
    let home = std::path::Path::new(&plan.spec.home);
    let store = agend_daemon::store::SqliteStore::open(home, 0).unwrap();
    fs::write(home.join("config.toml"), b"keep until shutdown").unwrap();
    assert!(owned.remove_with_data(&manager, true).is_err());
    assert_eq!(
        fs::read(home.join("config.toml")).unwrap(),
        b"keep until shutdown"
    );
    assert!(home.join("agend.db").exists());
    drop(store);
    owned.remove_with_data(&manager, true).unwrap();
    assert!(!home.join("agend.db").exists());
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires an owned disposable mount namespace with CAP_SYS_ADMIN"]
fn same_device_bind_mount_cannot_delete_external_data() {
    use std::os::unix::fs::MetadataExt;
    let (root, plan) = fixture();
    let manager = Model::new();
    let mut owned = Owned::prepare(&plan, &plan.source, None).unwrap();
    let foreign = root.path().join("external");
    let mount = std::path::Path::new(&plan.spec.home).join("bound-data");
    fs::create_dir(&foreign).unwrap();
    fs::create_dir(&mount).unwrap();
    fs::write(foreign.join("sentinel"), b"external data").unwrap();
    struct Unmount(std::path::PathBuf);
    impl Drop for Unmount {
        fn drop(&mut self) {
            let status = std::process::Command::new("umount")
                .arg(&self.0)
                .status()
                .unwrap();
            assert!(status.success(), "owned bind mount cleanup failed");
        }
    }
    assert!(
        std::process::Command::new("mount")
            .arg("--bind")
            .arg(&foreign)
            .arg(&mount)
            .status()
            .unwrap()
            .success()
    );
    let guard = Unmount(mount.clone());
    assert_eq!(
        fs::metadata(&foreign).unwrap().dev(),
        fs::metadata(&mount).unwrap().dev()
    );
    let error = owned.remove_with_data(&manager, true).unwrap_err();
    assert!(error.contains("mount boundary"), "{error}");
    assert_eq!(manager.stops.get(), 0);
    assert_eq!(
        fs::read(foreign.join("sentinel")).unwrap(),
        b"external data"
    );
    assert!(plan.service_path.exists());
    drop(guard);
}
