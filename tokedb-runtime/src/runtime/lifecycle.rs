use serde::{Deserialize, Serialize};

use crate::error::{Result, RuntimeError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ContainerState {
    Created,
    Starting,
    Running,
    Stopping,
    Stopped,
    Destroyed,
}

impl ContainerState {
    pub fn is_terminal(&self) -> bool {
        matches!(self, ContainerState::Destroyed)
    }

    pub fn transition(&self, target: ContainerState) -> Result<ContainerState> {
        let allowed = matches!(
            (self, target),
            (ContainerState::Created, ContainerState::Starting)
                | (ContainerState::Starting, ContainerState::Running)
                | (ContainerState::Starting, ContainerState::Stopping)
                | (ContainerState::Running, ContainerState::Stopping)
                | (ContainerState::Running, ContainerState::Stopped)
                | (ContainerState::Stopping, ContainerState::Stopped)
                | (ContainerState::Stopped, ContainerState::Starting)
                | (ContainerState::Stopped, ContainerState::Destroyed)
        );
        if allowed {
            Ok(target)
        } else {
            Err(RuntimeError::InvalidTransition {
                from: format!("{self:?}"),
                to: format!("{target:?}"),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ContainerState::*;

    const ALL: [ContainerState; 6] = [Created, Starting, Running, Stopping, Stopped, Destroyed];

    #[test]
    fn transitions_follow_defined_graph() {
        let allowed = [
            (Created, Starting),
            (Starting, Running),
            (Starting, Stopping),
            (Running, Stopping),
            (Running, Stopped),
            (Stopping, Stopped),
            (Stopped, Starting),
            (Stopped, Destroyed),
        ];
        for from in ALL {
            for to in ALL {
                let expected = allowed.contains(&(from, to));
                assert_eq!(
                    from.transition(to).is_ok(),
                    expected,
                    "transition {from:?} -> {to:?}"
                );
            }
        }
    }

    #[test]
    fn destroyed_is_terminal() {
        assert!(Destroyed.is_terminal());
        assert!(!Created.is_terminal());
        assert!(!Running.is_terminal());
    }

    #[test]
    fn invalid_transition_carries_context() {
        let err = Created.transition(Running).unwrap_err();
        assert!(matches!(
            err,
            RuntimeError::InvalidTransition { ref from, ref to }
                if from == "Created" && to == "Running"
        ));
    }

    #[test]
    fn state_serializes_to_lowercase() {
        for (state, expected) in [
            (Created, "created"),
            (Starting, "starting"),
            (Running, "running"),
            (Stopping, "stopping"),
            (Stopped, "stopped"),
            (Destroyed, "destroyed"),
        ] {
            assert_eq!(
                serde_json::to_string(&state).unwrap(),
                format!("\"{expected}\"")
            );
        }
    }
}
