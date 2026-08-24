use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Capability(u64);

impl Capability {
    pub const CHOWN: Capability = Capability(0);
    pub const DAC_OVERRIDE: Capability = Capability(1);
    pub const DAC_READ_SEARCH: Capability = Capability(2);
    pub const FOWNER: Capability = Capability(3);
    pub const FSETID: Capability = Capability(4);
    pub const KILL: Capability = Capability(5);
    pub const SETGID: Capability = Capability(6);
    pub const SETUID: Capability = Capability(7);
    pub const SETPCAP: Capability = Capability(8);
    pub const NET_BIND_SERVICE: Capability = Capability(10);
    pub const NET_RAW: Capability = Capability(13);
    pub const SYS_CHROOT: Capability = Capability(18);
    pub const SYS_PTRACE: Capability = Capability(19);
    pub const SYS_ADMIN: Capability = Capability(21);
    pub const MKNOD: Capability = Capability(27);
    pub const SYS_RESOURCE: Capability = Capability(24);

    pub fn index(self) -> u32 {
        self.0 as u32
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilitySet(u64);

impl CapabilitySet {
    pub fn allow(self, cap: Capability) -> Self {
        CapabilitySet(self.0 | (1 << cap.index()))
    }

    pub fn contains(self, cap: Capability) -> bool {
        self.0 & (1 << cap.index()) != 0
    }

    pub fn mask(self) -> u64 {
        self.0
    }
}

pub fn default_allowlist() -> CapabilitySet {
    CapabilitySet::default()
        .allow(Capability::CHOWN)
        .allow(Capability::DAC_OVERRIDE)
        .allow(Capability::FOWNER)
        .allow(Capability::SETGID)
        .allow(Capability::SETUID)
        .allow(Capability::NET_BIND_SERVICE)
}

#[cfg(target_os = "linux")]
pub fn restrict_to(set: &CapabilitySet) -> crate::error::Result<()> {
    linux_sys::restrict_caps(*set)
}

#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
mod linux_sys {
    use super::CapabilitySet;
    use crate::error::{Result, RuntimeError};

    const LINUX_CAPABILITY_VERSION_3: u32 = 0x2008_0522;
    const LAST_CAPABILITY: u64 = 40;

    #[repr(C)]
    struct CapHeader {
        version: u32,
        pid: i32,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CapData {
        effective: u32,
        permitted: u32,
        inheritable: u32,
    }

    pub(super) fn restrict_caps(set: CapabilitySet) -> Result<()> {
        for cap in 0..=LAST_CAPABILITY {
            if set.mask() & (1 << cap) != 0 {
                continue;
            }
            let rc = unsafe { libc::prctl(libc::PR_CAPBSET_DROP, cap) };
            if rc != 0 {
                return Err(RuntimeError::Process(format!(
                    "PR_CAPBSET_DROP({cap}): {}",
                    std::io::Error::last_os_error()
                )));
            }
        }
        let header = CapHeader {
            version: LINUX_CAPABILITY_VERSION_3,
            pid: 0,
        };
        let mask = set.mask();
        let low = mask as u32;
        let high = (mask >> 32) as u32;
        let data = [
            CapData {
                effective: low,
                permitted: low,
                inheritable: 0,
            },
            CapData {
                effective: high,
                permitted: high,
                inheritable: 0,
            },
        ];
        let rc = unsafe { libc::syscall(libc::SYS_capset, &header, data.as_ptr()) };
        if rc != 0 {
            return Err(RuntimeError::Process(format!(
                "capset(mask={:#x}): {}",
                set.mask(),
                std::io::Error::last_os_error()
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXPECTED_DEFAULT_MASK: u64 =
        (1 << 0) | (1 << 1) | (1 << 3) | (1 << 6) | (1 << 7) | (1 << 10);

    #[test]
    fn default_allowlist_matches_plan() {
        assert_eq!(default_allowlist().mask(), EXPECTED_DEFAULT_MASK);
    }

    #[test]
    fn empty_set_has_no_capabilities() {
        let set = CapabilitySet::default();
        assert_eq!(set.mask(), 0);
        assert!(!set.contains(Capability::CHOWN));
    }

    #[test]
    fn allow_and_contains_roundtrip() {
        let set = CapabilitySet::default()
            .allow(Capability::SETUID)
            .allow(Capability::NET_BIND_SERVICE);
        assert!(set.contains(Capability::SETUID));
        assert!(set.contains(Capability::NET_BIND_SERVICE));
        assert!(!set.contains(Capability::CHOWN));
    }

    #[test]
    fn capability_set_serde_roundtrip() {
        let set = default_allowlist();
        let value = serde_json::to_value(set).unwrap();
        let decoded: CapabilitySet = serde_json::from_value(value).unwrap();
        assert_eq!(decoded, set);
    }

    #[test]
    fn capability_indexes_are_stable() {
        assert_eq!(Capability::CHOWN.index(), 0);
        assert_eq!(Capability::DAC_OVERRIDE.index(), 1);
        assert_eq!(Capability::FOWNER.index(), 3);
        assert_eq!(Capability::SETGID.index(), 6);
        assert_eq!(Capability::SETUID.index(), 7);
        assert_eq!(Capability::NET_BIND_SERVICE.index(), 10);
    }
}
