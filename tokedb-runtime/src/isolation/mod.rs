pub mod capabilities;
pub mod cgroups;
pub mod privdrop;
pub mod seccomp;

use serde::{Deserialize, Serialize};

pub use capabilities::{default_allowlist, Capability, CapabilitySet};
pub use cgroups::{CgroupManager, ResourceLimits};
pub use privdrop::ContainerUser;
pub use seccomp::{SeccompProfile, SeccompSyscall};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SecurityProfile {
    pub capabilities: CapabilitySet,
    pub seccomp: Option<SeccompProfile>,
    pub user: Option<ContainerUser>,
}

#[cfg(target_os = "linux")]
pub fn apply_security(profile: &SecurityProfile) -> crate::error::Result<()> {
    use crate::error::RuntimeError;
    nix::sys::prctl::set_no_new_privs().map_err(RuntimeError::from)?;
    if let Some(seccomp) = &profile.seccomp {
        seccomp::apply_seccomp(seccomp)?;
    }
    capabilities::restrict_to(&profile.capabilities)?;
    if let Some(user) = &profile.user {
        privdrop::drop_privileges(user)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn security_profile_serde_roundtrip() {
        let profile = SecurityProfile {
            capabilities: default_allowlist(),
            seccomp: Some(SeccompProfile::default_denylist()),
            user: Some(ContainerUser {
                uid: 1000,
                gid: 1000,
            }),
        };
        let value = serde_json::to_value(&profile).unwrap();
        let decoded: SecurityProfile = serde_json::from_value(value).unwrap();
        assert_eq!(decoded, profile);
        assert_eq!(decoded.user.unwrap().uid, 1000);
        assert_eq!(decoded.seccomp.unwrap().blocked.len(), 19);
    }
}
