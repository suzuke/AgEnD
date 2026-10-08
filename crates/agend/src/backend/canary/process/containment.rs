//! Version probes may create threads but no descendant processes. This is a
//! process-lifecycle restriction, not a filesystem or network sandbox.
use std::{io, path::Path, process::Command};

pub(super) fn command(program: &Path) -> io::Result<Command> {
    #[cfg(target_os = "macos")]
    {
        let mut command = Command::new("/usr/bin/sandbox-exec");
        command.args(["-p", "(version 1) (allow default) (deny process-fork)"]);
        command.arg(program);
        Ok(command)
    }
    #[cfg(all(
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64")
    ))]
    {
        use std::os::unix::process::CommandExt;
        let mut command = Command::new(program);
        // SAFETY: install uses fixed stack data and async-signal-safe syscalls.
        unsafe {
            command.pre_exec(linux::install);
        }
        Ok(command)
    }
    #[cfg(not(any(
        target_os = "macos",
        all(
            target_os = "linux",
            any(target_arch = "x86_64", target_arch = "aarch64")
        )
    )))]
    {
        let _ = program;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "version probe process containment is unavailable",
        ))
    }
}

#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
mod linux {
    use std::io;
    const fn stmt(code: u16, k: u32) -> libc::sock_filter {
        libc::sock_filter {
            code,
            jt: 0,
            jf: 0,
            k,
        }
    }
    const fn eq(k: u32, yes: u8, no: u8) -> libc::sock_filter {
        libc::sock_filter {
            code: 0x15,
            jt: yes,
            jf: no,
            k,
        }
    }
    #[cfg(target_arch = "x86_64")]
    const ARCH: u32 = 0xc000003e;
    #[cfg(target_arch = "aarch64")]
    const ARCH: u32 = 0xc00000b7;

    pub(super) fn install() -> io::Result<()> {
        // seccomp_data: nr at 0, arch at 4, low 32 bits of args[0] at 16
        // on both supported little-endian ABIs. Never inspect clone3 pointers:
        // ENOSYS makes libc fall back to clone, whose flags are filterable.
        let filter = [
            stmt(0x20, 4),
            eq(ARCH, 1, 0),
            stmt(0x06, 0x80000000),
            stmt(0x20, 0),
            // x32 shares the audit architecture; refuse its syscall namespace.
            libc::sock_filter {
                code: 0x45,
                jt: 0,
                jf: 1,
                k: 0x40000000,
            },
            stmt(0x06, 0x00050000 | libc::ENOSYS as u32),
            #[cfg(target_arch = "x86_64")]
            eq(libc::SYS_fork as u32, 0, 1),
            #[cfg(target_arch = "x86_64")]
            stmt(0x06, 0x00050000 | libc::EPERM as u32),
            #[cfg(target_arch = "x86_64")]
            eq(libc::SYS_vfork as u32, 0, 1),
            #[cfg(target_arch = "x86_64")]
            stmt(0x06, 0x00050000 | libc::EPERM as u32),
            eq(libc::SYS_clone3 as u32, 0, 1),
            stmt(0x06, 0x00050000 | libc::ENOSYS as u32),
            eq(libc::SYS_clone as u32, 0, 3),
            stmt(0x20, 16),
            libc::sock_filter {
                code: 0x45,
                jt: 1,
                jf: 0,
                k: libc::CLONE_THREAD as u32,
            },
            stmt(0x06, 0x00050000 | libc::EPERM as u32),
            stmt(0x06, 0x7fff0000),
        ];
        let program = libc::sock_fprog {
            len: filter.len() as u16,
            filter: filter.as_ptr() as *mut libc::sock_filter,
        };
        // SAFETY: these prctl calls affect only this fork child; the kernel
        // copies the valid filter array before returning. No allocation or lock.
        if unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0
            || unsafe { libc::prctl(libc::PR_SET_SECCOMP, 2, &program) } != 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}
