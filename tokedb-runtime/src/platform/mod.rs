#[cfg(target_os = "linux")]
mod linux;
#[cfg(not(target_os = "linux"))]
mod unsupported;

pub trait Platform: Send + Sync {
    fn name(&self) -> &'static str;
    fn supports_containers(&self) -> bool;
}

#[cfg(target_os = "linux")]
pub type ActivePlatform = linux::LinuxPlatform;
#[cfg(not(target_os = "linux"))]
pub type ActivePlatform = unsupported::UnsupportedPlatform;

pub fn current() -> ActivePlatform {
    ActivePlatform::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_has_consistent_identity() {
        let platform = current();
        assert!(!platform.name().is_empty());
    }
}
