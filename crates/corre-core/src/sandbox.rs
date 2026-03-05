//! Landlock + seccomp sandbox for app subprocesses.
//!
//! Applies filesystem and network restrictions via a `pre_exec` hook on the
//! child process `Command`. The parent process is unaffected. Works as an
//! unprivileged user inside Docker without any special container capabilities.

use corre_sdk::manifest::SandboxPermissions;
use std::path::{Path, PathBuf};
use tokio::process::Command;

/// Sandbox that applies Landlock filesystem restrictions and seccomp network
/// filtering to a child process via `pre_exec`.
pub struct LandlockSandbox {
    ro_paths: Vec<PathBuf>,
    rw_paths: Vec<PathBuf>,
    allow_network: bool,
}

impl LandlockSandbox {
    /// Build a `LandlockSandbox` from [`SandboxPermissions`].
    ///
    /// Template variables `{data_dir}`, `{config_dir}`, and `{plugin_dir}` in
    /// the permission paths are expanded to their actual values.
    ///
    /// - `{config_dir}` expands to `{data_dir}/{app_name}/config/`
    /// - The app's home dir (`{data_dir}/{app_name}/`) is
    ///   automatically granted read-write access.
    pub fn from_permissions(perms: &SandboxPermissions, plugin_dir: &Path, data_dir: &Path, app_name: &str) -> Self {
        let app_home = data_dir.join(app_name);
        let config_dir = app_home.join("config");
        let data_dir_str = data_dir.to_string_lossy();
        let config_dir_str = config_dir.to_string_lossy();
        let plugin_dir_str = plugin_dir.to_string_lossy();

        let expand = |s: &str| -> String {
            s.replace("{data_dir}", &data_dir_str).replace("{config_dir}", &config_dir_str).replace("{plugin_dir}", &plugin_dir_str)
        };

        let mut ro_paths = vec![PathBuf::from("/usr"), PathBuf::from("/lib"), PathBuf::from("/lib64")];

        for path in &perms.filesystem_read {
            ro_paths.push(PathBuf::from(expand(path)));
        }

        let mut rw_paths = vec![plugin_dir.to_path_buf(), app_home];

        for path in &perms.filesystem_write {
            rw_paths.push(PathBuf::from(expand(path)));
        }

        let has_network = !perms.network.is_empty();
        let dns = perms.dns.unwrap_or(has_network);
        if dns {
            ro_paths.push(PathBuf::from("/etc/resolv.conf"));
            ro_paths.push(PathBuf::from("/etc/nsswitch.conf"));
        }

        Self { ro_paths, rw_paths, allow_network: has_network }
    }

    /// Apply sandbox restrictions to a [`Command`] via `env_clear` and `pre_exec`.
    ///
    /// The `pre_exec` closure runs in the forked child before `exec`, so
    /// restrictions apply only to the app binary and its children.
    pub fn apply_to_command(&self, cmd: &mut Command) {
        cmd.env_clear();

        let ro_paths = self.ro_paths.clone();
        let rw_paths = self.rw_paths.clone();
        let allow_network = self.allow_network;

        // SAFETY: The closure runs between fork() and exec() in the child
        // process. We only call async-signal-safe operations (prctl, seccomp)
        // and the landlock crate's restrict_self which uses syscalls directly.
        unsafe {
            cmd.pre_exec(move || {
                // 1. Prevent privilege escalation via setuid binaries
                let ret = libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0);
                if ret != 0 {
                    return Err(std::io::Error::last_os_error());
                }

                // 2. Apply Landlock filesystem restrictions
                if let Err(e) = apply_landlock(&ro_paths, &rw_paths) {
                    // Log is not available in pre_exec (post-fork), so we
                    // write directly to stderr. If Landlock is unsupported
                    // (old kernel), we continue — seccomp still provides
                    // network isolation independently.
                    let _ = write_stderr(&format!("landlock: {e}\n"));
                }

                // 3. Apply seccomp network filter if network is denied
                if !allow_network {
                    if let Err(e) = install_seccomp_net_filter() {
                        return Err(std::io::Error::new(std::io::ErrorKind::Other, format!("seccomp: {e}")));
                    }
                }

                Ok(())
            });
        }
    }
}

/// Apply Landlock filesystem restrictions using best-effort ABI negotiation.
fn apply_landlock(ro_paths: &[PathBuf], rw_paths: &[PathBuf]) -> Result<(), landlock::RulesetError> {
    #[allow(unused_imports)]
    use landlock::RestrictionStatus;
    use landlock::{ABI, Access, AccessFs, PathBeneath, PathFd, Ruleset, RulesetAttr, RulesetCreatedAttr, RulesetStatus};

    let abi = ABI::V2;

    let read_access = AccessFs::from_read(abi);
    let read_write_access = AccessFs::from_all(abi);

    let mut ruleset = Ruleset::default().handle_access(read_write_access)?.create()?;

    for path in ro_paths {
        if path.exists() {
            match PathFd::new(path) {
                Ok(fd) => {
                    ruleset = ruleset.add_rule(PathBeneath::new(fd, read_access))?;
                }
                Err(_) => continue,
            }
        }
    }

    for path in rw_paths {
        if path.exists() {
            match PathFd::new(path) {
                Ok(fd) => {
                    ruleset = ruleset.add_rule(PathBeneath::new(fd, read_write_access))?;
                }
                Err(_) => continue,
            }
        }
    }

    let status = ruleset.restrict_self()?;
    match status.ruleset {
        RulesetStatus::NotEnforced => {
            let _ = write_stderr("landlock: not supported on this kernel, continuing without filesystem sandbox\n");
        }
        RulesetStatus::PartiallyEnforced => {
            let _ = write_stderr("landlock: partially compatible ABI, some restrictions may not apply\n");
        }
        RulesetStatus::FullyEnforced => {}
    }

    Ok(())
}

/// Install a seccomp-BPF filter that blocks network-related syscalls with EPERM.
fn install_seccomp_net_filter() -> Result<(), String> {
    // Syscall numbers for x86_64
    #[cfg(target_arch = "x86_64")]
    const BLOCKED_SYSCALLS: &[u32] = &[
        41,  // SYS_socket
        42,  // SYS_connect
        44,  // SYS_sendto
        49,  // SYS_bind
        50,  // SYS_listen
        43,  // SYS_accept
        288, // SYS_accept4
    ];

    #[cfg(target_arch = "aarch64")]
    const BLOCKED_SYSCALLS: &[u32] = &[
        198, // SYS_socket
        203, // SYS_connect
        206, // SYS_sendto
        200, // SYS_bind
        201, // SYS_listen
        202, // SYS_accept
        242, // SYS_accept4
    ];

    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    compile_error!("seccomp filter: unsupported architecture");

    const SECCOMP_SET_MODE_FILTER: libc::c_ulong = 1;
    const SECCOMP_RET_ALLOW: u32 = 0x7fff_0000;
    const SECCOMP_RET_ERRNO: u32 = 0x0005_0000;

    // BPF constants
    const BPF_LD: u16 = 0x00;
    const BPF_W: u16 = 0x00;
    const BPF_ABS: u16 = 0x20;
    const BPF_JMP: u16 = 0x05;
    const BPF_JEQ: u16 = 0x10;
    const BPF_K: u16 = 0x00;
    const BPF_RET: u16 = 0x06;

    #[repr(C)]
    struct SockFilter {
        code: u16,
        jt: u8,
        jf: u8,
        k: u32,
    }

    #[repr(C)]
    struct SockFprog {
        len: u16,
        filter: *const SockFilter,
    }

    // Build BPF program:
    //   [0] load syscall number (offset 0 in seccomp_data)
    //   [1..N] for each blocked syscall: JEQ → return ERRNO(EPERM)
    //   [N+1] return ALLOW
    let mut filter: Vec<SockFilter> = Vec::with_capacity(2 + BLOCKED_SYSCALLS.len() * 2);

    // Load the syscall number from seccomp_data.nr (offset 0)
    filter.push(SockFilter { code: BPF_LD | BPF_W | BPF_ABS, jt: 0, jf: 0, k: 0 });

    // Layout: [0] LD nr, [1..N] JEQ → RET_ERRNO, [N+1] RET_ALLOW, [N+2] RET_ERRNO
    // From JEQ at position (1+i), RET_ERRNO is at (num_blocked + 2) → jt = num_blocked - i
    let num_blocked = BLOCKED_SYSCALLS.len();
    for (i, &syscall) in BLOCKED_SYSCALLS.iter().enumerate() {
        let jt = (num_blocked - i) as u8;
        filter.push(SockFilter { code: BPF_JMP | BPF_JEQ | BPF_K, jt, jf: 0, k: syscall });
    }

    // Default: allow
    filter.push(SockFilter { code: BPF_RET | BPF_K, jt: 0, jf: 0, k: SECCOMP_RET_ALLOW });

    // Block: return EPERM
    filter.push(SockFilter { code: BPF_RET | BPF_K, jt: 0, jf: 0, k: SECCOMP_RET_ERRNO | (libc::EPERM as u32) });

    let prog = SockFprog { len: u16::try_from(filter.len()).expect("BPF program too large for u16"), filter: filter.as_ptr() };

    let ret = unsafe { libc::syscall(libc::SYS_seccomp, SECCOMP_SET_MODE_FILTER, 0u64, &prog as *const SockFprog) };

    if ret != 0 {
        return Err(format!("SYS_seccomp failed: {}", std::io::Error::last_os_error()));
    }

    Ok(())
}

/// Write a message to stderr directly (safe in post-fork pre_exec context).
fn write_stderr(msg: &str) -> Result<(), std::io::Error> {
    unsafe {
        let ret = libc::write(2, msg.as_ptr() as *const libc::c_void, msg.len());
        if ret < 0 { Err(std::io::Error::last_os_error()) } else { Ok(()) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use corre_sdk::manifest::SandboxPermissions;

    #[test]
    fn landlock_sandbox_from_permissions() {
        let perms = SandboxPermissions {
            network: vec!["api.venice.ai:443".into()],
            filesystem_read: vec!["{config_dir}".into()],
            filesystem_write: vec![],
            dns: None,
            max_memory_mb: None,
            max_cpu_secs: None,
        };

        let sandbox = LandlockSandbox::from_permissions(
            &perms,
            Path::new("/home/user/.local/share/corre/plugins/daily-brief"),
            Path::new("/home/user/.local/share/corre"),
            "daily-brief",
        );

        assert!(sandbox.allow_network);
        // ro_paths: /usr, /lib, /lib64, config path, resolv.conf, nsswitch.conf
        assert!(sandbox.ro_paths.len() >= 5);
        assert!(sandbox.ro_paths.iter().any(|p| p.ends_with("daily-brief/config")));
        // rw_paths: plugin_dir + app home
        assert!(sandbox.rw_paths.len() >= 2);
        assert!(sandbox.rw_paths.iter().any(|p| p.ends_with("daily-brief")));
    }

    #[test]
    fn landlock_sandbox_no_network() {
        let perms = SandboxPermissions::default();
        let sandbox = LandlockSandbox::from_permissions(&perms, Path::new("/tmp/plugin"), Path::new("/tmp/data"), "test-cap");
        assert!(!sandbox.allow_network);
    }

    #[test]
    fn landlock_sandbox_env_cleared() {
        let perms = SandboxPermissions::default();
        let sandbox = LandlockSandbox::from_permissions(&perms, Path::new("/tmp/plugin"), Path::new("/tmp/data"), "test-cap");
        let mut cmd = Command::new("/bin/true");
        sandbox.apply_to_command(&mut cmd);
        // Command was constructed without error — env_clear and pre_exec were applied.
        // (We can't inspect the internal state of Command, but the call didn't panic.)
    }
}
