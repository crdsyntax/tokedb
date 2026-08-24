use super::Platform;

#[derive(Debug, Clone, Copy, Default)]
pub struct LinuxPlatform;

impl Platform for LinuxPlatform {
    fn name(&self) -> &'static str {
        "linux"
    }

    fn supports_containers(&self) -> bool {
        true
    }
}
