//! Match the actual process image, not only its current pathname.
use std::os::unix::fs::MetadataExt;
use std::path::Path;

#[cfg(target_os = "linux")]
pub(super) fn matches(path: &Path) -> Result<(), String> {
    let loaded = std::fs::metadata("/proc/self/exe").map_err(|e| e.to_string())?;
    let expected = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if loaded.dev() != expected.dev() || loaded.ino() != expected.ino() {
        return Err("AgEnD pathname does not identify the running daemon image".into());
    }
    Ok(())
}

#[cfg(target_os = "macos")]
#[repr(C)]
struct Region {
    protection: u32,
    attributes: [u32; 3],
    offset: u64,
    accounting: [u32; 14],
    address: u64,
    size: u64,
}
#[cfg(target_os = "macos")]
#[repr(C)]
struct RegionPath {
    region: Region,
    vnode: libc::vnode_info_path,
}

#[cfg(target_os = "macos")]
pub(super) fn matches(path: &Path) -> Result<(), String> {
    let expected = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    let mut address = 0u64;
    for _ in 0..256 {
        let mut info = std::mem::MaybeUninit::<RegionPath>::uninit();
        let size = std::mem::size_of::<RegionPath>() as i32;
        // SAFETY: flavor 8 writes the SDK proc_regionwithpathinfo layout above.
        // The exact initialized size is required before accessing its fields.
        let count = unsafe {
            libc::proc_pidinfo(
                std::process::id() as i32,
                8,
                address,
                info.as_mut_ptr().cast(),
                size,
            )
        };
        if count != size {
            break;
        }
        // SAFETY: the native call returned the complete structure.
        let info = unsafe { info.assume_init() };
        if info.region.protection & libc::VM_PROT_EXECUTE as u32 != 0
            && info.vnode.vip_path[0][0] != 0
        {
            let stat = info.vnode.vip_vi.vi_stat;
            return if (u64::from(stat.vst_dev), stat.vst_ino) == (expected.dev(), expected.ino()) {
                Ok(())
            } else {
                Err("AgEnD pathname does not identify the running daemon image".into())
            };
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
    Err("cannot establish running AgEnD executable mapping".into())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub(super) fn matches(_: &Path) -> Result<(), String> {
    Err("running executable verification is unavailable on this platform".into())
}
