use serde::{Deserialize, Serialize};

use crate::MachineConnection;

/// A named Ployz context and its ordered connection choices.
#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
pub struct Context {
    /// Runtime-only context name; the containing config map persists the name.
    #[serde(skip)]
    pub name: String,
    /// Connections in preference order.
    #[serde(default)]
    pub connections: Vec<MachineConnection>,
}

impl Context {
    /// Moves the selected connection to the front, leaving all others ordered.
    pub fn set_default_connection(&mut self, index: isize) {
        let Ok(index) = usize::try_from(index) else {
            return;
        };
        if index >= self.connections.len() {
            return;
        }
        self.connections[..=index].rotate_right(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connection(name: &str) -> MachineConnection {
        MachineConnection {
            ssh: name.into(),
            ..Default::default()
        }
    }

    #[test]
    fn set_default_connection_moves_only_selected_connection_to_front() {
        let mut context = Context {
            connections: vec![connection("a"), connection("b"), connection("c")],
            ..Default::default()
        };

        context.set_default_connection(2);

        let destinations: Vec<_> = context
            .connections
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(destinations, ["ssh://c", "ssh://a", "ssh://b"]);
    }

    #[test]
    fn set_default_connection_is_noop_for_first_or_out_of_range_index() {
        let original = vec![connection("a"), connection("b")];
        for index in [0, -1, 2, isize::MAX] {
            let mut context = Context {
                connections: original.clone(),
                ..Default::default()
            };
            context.set_default_connection(index);
            assert_eq!(context.connections, original);
        }
    }
}
