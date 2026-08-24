use super::Platform;

#[derive(Debug, Clone, Copy, Default)]
pub struct UnsupportedPlatform;

impl Platform for UnsupportedPlatform {
    fn name(&self) -> &'static str {
        std::env::consts::OS
    }

    fn supports_containers(&self) -> bool {
        false
    }
}
