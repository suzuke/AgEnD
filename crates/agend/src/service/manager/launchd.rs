//! Native process identity for a launchd-owned daemon. Never return environment
//! bytes in errors: they can contain credentials unrelated to AgEnD.

use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use agend_core::setup::service::Installation;

pub(super) fn live_process(record: &Installation, pid: u32) -> Result<(), String> {
    let pid = i32::try_from(pid)
        .ok()
        .filter(|pid| *pid > 1)
        .ok_or("invalid launchd PID")?;
    let before = identity(pid)?;
    let mut path = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    // SAFETY: path is a writable buffer with the advertised capacity.
    let count = unsafe { libc::proc_pidpath(pid, path.as_mut_ptr().cast(), path.len() as u32) };
    if count <= 0 {
        return Err("cannot inspect live launchd executable; preserving service".into());
    }
    let end = path
        .iter()
        .position(|&byte| byte == 0)
        .ok_or("invalid executable path")?;
    let live = fs::metadata(Path::new(std::ffi::OsStr::from_bytes(&path[..end])))
        .map_err(|_| "cannot stat live launchd executable")?;
    let expected =
        fs::metadata(&record.spec.program).map_err(|_| "managed executable is missing")?;
    if (live.dev(), live.ino()) != (expected.dev(), expected.ino()) {
        return Err("live launchd executable is not owned; preserving service".into());
    }
    mapped_executable(pid, &expected)?;
    let bytes = process_args(pid)?;
    let (args, env) = decode(&bytes)?;
    if args != [record.spec.program.as_bytes(), b"daemon"] {
        return Err("live launchd arguments are not owned; preserving service".into());
    }
    let homes: Vec<_> = env
        .iter()
        .filter_map(|value| value.strip_prefix(b"AGEND_HOME="))
        .collect();
    if homes != [record.spec.home.as_bytes()] {
        return Err("live launchd home is not owned; preserving service".into());
    }
    if identity(pid)? != before {
        return Err("launchd process changed during inspection; retry".into());
    }
    Ok(())
}

// ABI from Apple's SDK <sys/proc_info.h>: proc_regioninfo and
// proc_regionwithpathinfo. libc supplies vnode_info_path but not these types.
#[repr(C)]
struct Region {
    protection: u32,
    attributes: [u32; 3],
    offset: u64,
    accounting: [u32; 14],
    address: u64,
    size: u64,
}
#[repr(C)]
struct RegionPath {
    region: Region,
    vnode: libc::vnode_info_path,
}

fn mapped_executable(pid: i32, expected: &fs::Metadata) -> Result<(), String> {
    let mut address = 0u64;
    for _ in 0..256 {
        let mut info = std::mem::MaybeUninit::<RegionPath>::uninit();
        let size = std::mem::size_of::<RegionPath>() as i32;
        // SAFETY: PROC_PIDREGIONPATHINFO (8) writes the SDK-compatible buffer;
        // successful exact-size return is required before reading it.
        let count = unsafe { libc::proc_pidinfo(pid, 8, address, info.as_mut_ptr().cast(), size) };
        if count != size {
            break;
        }
        // SAFETY: the native call initialized the complete output above.
        let info = unsafe { info.assume_init() };
        if info.region.protection & libc::VM_PROT_EXECUTE as u32 != 0
            && info.vnode.vip_path[0][0] != 0
        {
            let stat = info.vnode.vip_vi.vi_stat;
            if (u64::from(stat.vst_dev), stat.vst_ino) == (expected.dev(), expected.ino()) {
                return Ok(());
            }
            return Err("live launchd mapped executable is not owned; preserving service".into());
        }
        let next = info
            .region
            .address
            .checked_add(info.region.size)
            .ok_or("invalid mapped region")?;
        if next <= address {
            break;
        }
        address = next;
    }
    Err("cannot establish live launchd executable mapping; preserving service".into())
}

fn identity(pid: i32) -> Result<(u64, u64), String> {
    let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::uninit();
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as i32;
    // SAFETY: the output buffer has the requested proc_bsdinfo size/alignment.
    let count = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTBSDINFO,
            0,
            info.as_mut_ptr().cast(),
            size,
        )
    };
    if count != size {
        return Err("cannot inspect launchd process generation; preserving service".into());
    }
    // SAFETY: the native call initialized the whole structure.
    let info = unsafe { info.assume_init() };
    // SAFETY: getuid only reads this process's uid.
    let uid = unsafe { libc::getuid() };
    if info.pbi_pid != pid as u32 || info.pbi_uid != uid || info.pbi_ruid != uid {
        return Err("launchd process belongs to another user; preserving service".into());
    }
    Ok((info.pbi_start_tvsec, info.pbi_start_tvusec))
}

fn process_args(pid: i32) -> Result<Vec<u8>, String> {
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid];
    // Bounded by design; fail closed instead of allocating a size controlled by
    // another process. sysctl updates size to the actual initialized byte count.
    let mut bytes = vec![0u8; 1024 * 1024];
    let mut size = bytes.len();
    // SAFETY: mib and bytes are valid writable buffers of their given lengths.
    let result = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            3,
            bytes.as_mut_ptr().cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if result != 0 || size > bytes.len() {
        return Err("cannot inspect launchd arguments/environment; preserving service".into());
    }
    bytes.truncate(size);
    Ok(bytes)
}

type Arguments<'a> = (Vec<&'a [u8]>, Vec<&'a [u8]>);
fn decode(bytes: &[u8]) -> Result<Arguments<'_>, String> {
    let argc = bytes.get(..4).ok_or("short process arguments")?;
    let argc = i32::from_ne_bytes(argc.try_into().map_err(|_| "invalid argc")?);
    if argc != 2 {
        return Err("live launchd argument count is not owned".into());
    }
    let mut rest = &bytes[4..];
    take_string(&mut rest)?; // Executable path, followed by NUL padding.
    while rest.first() == Some(&0) {
        rest = &rest[1..];
    }
    let args = (0..argc)
        .map(|_| take_string(&mut rest))
        .collect::<Result<Vec<_>, _>>()?;
    let env = rest
        .split(|&byte| byte == 0)
        .filter(|value| !value.is_empty())
        .collect();
    Ok((args, env))
}

fn take_string<'a>(rest: &mut &'a [u8]) -> Result<&'a [u8], String> {
    let end = rest
        .iter()
        .position(|&byte| byte == 0)
        .ok_or("unterminated process argument")?;
    let value = &rest[..end];
    *rest = &rest[end + 1..];
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use agend_core::setup::service::{InstallPhase, Manager, ServiceSpec};
    use agend_testkit::tempdir::TempDir;
    use std::os::unix::process::CommandExt;
    use std::process::{Child, Command, Stdio};
    use std::time::{Duration, Instant};

    struct OwnedChild(Child);
    impl Drop for OwnedChild {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    #[test]
    fn native_process_identity_checks_executable_argv_home_and_exit() {
        let root = TempDir::new("g13-launchd-process").unwrap();
        let home = root.path().canonicalize().unwrap();
        let program = home.join("agend");
        let source = home.join("fixture.c");
        fs::write(
            &source,
            br#"#include <stdio.h>
#include <unistd.h>
#include <string.h>
#include <stddef.h>
#include <sys/proc_info.h>
int main(int argc, char **argv) {
    if (argc == 2 && !strcmp(argv[1], "layout")) {
        printf("%zu %zu %zu %zu %d", sizeof(struct proc_regioninfo),
            sizeof(struct proc_regionwithpathinfo), offsetof(struct proc_regioninfo, pri_address),
            offsetof(struct proc_regionwithpathinfo, prp_vip), PROC_PIDREGIONPATHINFO);
        return 0;
    }
    FILE *f=fopen("ready","w"); if(!f)return 1; fclose(f);
    char c; return read(0,&c,1)<0;
}
"#,
        )
        .unwrap();
        assert!(
            Command::new("/usr/bin/cc")
                .arg(&source)
                .arg("-o")
                .arg(&program)
                .status()
                .unwrap()
                .success()
        );
        let layout = Command::new(&program).arg("layout").output().unwrap();
        assert!(layout.status.success());
        assert_eq!(
            String::from_utf8(layout.stdout).unwrap(),
            format!(
                "{} {} {} {} 8",
                std::mem::size_of::<Region>(),
                std::mem::size_of::<RegionPath>(),
                std::mem::offset_of!(Region, address),
                std::mem::offset_of!(RegionPath, vnode)
            )
        );
        let mut child = OwnedChild(
            Command::new(&program)
                .arg("daemon")
                .current_dir(&home)
                .env_clear()
                .env("AGEND_HOME", &home)
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap(),
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        while !home.join("ready").exists() {
            let status = child.0.try_wait().unwrap();
            assert!(
                status.is_none(),
                "fixture exited before readiness: {status:?}"
            );
            assert!(Instant::now() < deadline, "fixture readiness timed out");
            std::thread::sleep(Duration::from_millis(10));
        }
        let mut record = Installation {
            version: 1,
            spec: ServiceSpec {
                manager: Manager::Launchd,
                program: program.to_str().unwrap().into(),
                home: home.to_str().unwrap().into(),
                user_home: home.to_str().unwrap().into(),
                search_path: "/usr/bin:/bin".into(),
            },
            service_path: "unused".into(),
            program_sha256: "unused".into(),
            phase: InstallPhase::Registered,
        };
        live_process(&record, child.0.id()).unwrap();
        record.spec.home.push_str("-foreign");
        assert!(
            live_process(&record, child.0.id())
                .unwrap_err()
                .contains("home")
        );
        record.spec.home = home.to_str().unwrap().into();
        record.spec.program = "/bin/sh".into();
        assert!(
            live_process(&record, child.0.id())
                .unwrap_err()
                .contains("executable")
        );
        record.spec.program = program.to_str().unwrap().into();
        let replacement = home.join("replacement");
        fs::copy(&program, &replacement).unwrap();
        fs::rename(&replacement, &program).unwrap();
        assert!(live_process(&record, child.0.id()).is_err());
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "identity refusal must not kill the process"
        );
        child.0.kill().unwrap();
        child.0.wait().unwrap();
        assert!(live_process(&record, child.0.id()).is_err());
        fs::remove_file(home.join("ready")).unwrap();
        let mut other = OwnedChild(
            Command::new(&program)
                .arg0("foreign-argv0")
                .arg("daemon")
                .current_dir(&home)
                .env_clear()
                .env("AGEND_HOME", &home)
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        while !home.join("ready").exists() {
            assert!(other.0.try_wait().unwrap().is_none());
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            live_process(&record, other.0.id())
                .unwrap_err()
                .contains("arguments")
        );
    }
}
