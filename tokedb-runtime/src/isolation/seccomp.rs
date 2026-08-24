use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SeccompSyscall {
    Ptrace,
    Chmod,
    Fchmodat,
    Chown,
    Kill,
    ProcessVmReadv,
    ProcessVmWritev,
    KexecLoad,
    Reboot,
    Mount,
    Umount2,
    PivotRoot,
    Chroot,
    InitModule,
    FinitModule,
    DeleteModule,
    Bpf,
    Userfaultfd,
    Swapon,
    Swapoff,
    Acct,
    Ioperm,
    Iopl,
}

#[cfg(target_os = "linux")]
impl SeccompSyscall {
    pub fn number(self) -> i64 {
        use SeccompSyscall::*;
        match self {
            Ptrace => libc::SYS_ptrace,
            Chmod => libc::SYS_chmod,
            Fchmodat => libc::SYS_fchmodat,
            Chown => libc::SYS_chown,
            Kill => libc::SYS_kill,
            ProcessVmReadv => libc::SYS_process_vm_readv,
            ProcessVmWritev => libc::SYS_process_vm_writev,
            KexecLoad => libc::SYS_kexec_load,
            Reboot => libc::SYS_reboot,
            Mount => libc::SYS_mount,
            Umount2 => libc::SYS_umount2,
            PivotRoot => libc::SYS_pivot_root,
            Chroot => libc::SYS_chroot,
            InitModule => libc::SYS_init_module,
            FinitModule => libc::SYS_finit_module,
            DeleteModule => libc::SYS_delete_module,
            Bpf => libc::SYS_bpf,
            Userfaultfd => libc::SYS_userfaultfd,
            Swapon => libc::SYS_swapon,
            Swapoff => libc::SYS_swapoff,
            Acct => libc::SYS_acct,
            Ioperm => libc::SYS_ioperm,
            Iopl => libc::SYS_iopl,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeccompProfile {
    pub blocked: Vec<SeccompSyscall>,
}

impl SeccompProfile {
    pub fn default_denylist() -> Self {
        use SeccompSyscall::*;
        SeccompProfile {
            blocked: vec![
                Ptrace,
                ProcessVmReadv,
                ProcessVmWritev,
                KexecLoad,
                Reboot,
                Mount,
                Umount2,
                PivotRoot,
                Chroot,
                InitModule,
                FinitModule,
                DeleteModule,
                Bpf,
                Userfaultfd,
                Swapon,
                Swapoff,
                Acct,
                Ioperm,
                Iopl,
            ],
        }
    }
}

#[cfg(target_os = "linux")]
pub fn apply_seccomp(profile: &SeccompProfile) -> crate::error::Result<()> {
    use std::collections::BTreeMap;
    use std::convert::TryInto;

    use seccompiler::{BpfProgram, SeccompAction, SeccompFilter, TargetArch};

    use crate::error::RuntimeError;

    let arch = TargetArch::try_from(std::env::consts::ARCH)
        .map_err(|_| RuntimeError::UnsupportedPlatform("seccomp architecture"))?;
    let entries: BTreeMap<i64, Vec<seccompiler::SeccompRule>> = profile
        .blocked
        .iter()
        .map(|syscall| (syscall.number(), Vec::new()))
        .collect();
    let filter = SeccompFilter::new(
        entries,
        SeccompAction::Allow,
        SeccompAction::Errno(libc::EPERM as u32),
        arch,
    )
    .map_err(|err| RuntimeError::Process(format!("seccomp compile: {err}")))?;
    let program: BpfProgram = filter
        .try_into()
        .map_err(|err| RuntimeError::Process(format!("seccomp bpf: {err}")))?;
    seccompiler::apply_filter(&program)
        .map_err(|err| RuntimeError::Process(format!("seccomp apply: {err}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_denylist_blocks_core_attack_surface() {
        use SeccompSyscall::*;
        let profile = SeccompProfile::default_denylist();
        for syscall in [
            Ptrace, Mount, Umount2, PivotRoot, Chroot, Bpf, KexecLoad, Reboot,
        ] {
            assert!(profile.blocked.contains(&syscall), "missing {syscall:?}");
        }
    }

    #[test]
    fn seccomp_profile_serde_roundtrip() {
        let profile = SeccompProfile {
            blocked: vec![SeccompSyscall::Ptrace, SeccompSyscall::Mount],
        };
        let value = serde_json::to_value(&profile).unwrap();
        let decoded: SeccompProfile = serde_json::from_value(value).unwrap();
        assert_eq!(decoded, profile);
        assert_eq!(
            serde_json::to_string(&SeccompSyscall::Ptrace).unwrap(),
            "\"ptrace\""
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn syscall_numbers_match_libc() {
        assert_eq!(SeccompSyscall::Ptrace.number(), libc::SYS_ptrace);
        assert_eq!(SeccompSyscall::Mount.number(), libc::SYS_mount);
        assert_eq!(SeccompSyscall::Bpf.number(), libc::SYS_bpf);
    }
}
