use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerUser {
    pub uid: u32,
    pub gid: u32,
}

#[cfg(target_os = "linux")]
pub fn drop_privileges(user: &ContainerUser) -> crate::error::Result<()> {
    use nix::unistd::{setgid, setgroups, setuid, Gid, Uid};

    use crate::error::RuntimeError;

    setgroups(&[]).map_err(RuntimeError::from)?;
    setgid(Gid::from_raw(user.gid)).map_err(RuntimeError::from)?;
    setuid(Uid::from_raw(user.uid)).map_err(RuntimeError::from)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn container_user_serde_roundtrip() {
        let user = ContainerUser {
            uid: 1000,
            gid: 1000,
        };
        let value = serde_json::to_value(user).unwrap();
        let decoded: ContainerUser = serde_json::from_value(value).unwrap();
        assert_eq!(decoded, user);
    }
}
