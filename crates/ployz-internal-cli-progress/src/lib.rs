//! Stable progress-event labels shared by client operations.
//!
//! Request cancellation and other execution state remain the caller's concern.
//! This crate carries only the optional event-ID override that downstream
//! operations inherit when they should update one existing progress line.

use std::sync::Arc;

use ployz_internal_cli_tui::FAINT;

/// An inherited override for the event ID chosen by a client operation.
///
/// An empty override is retained but intentionally treated as absent when an
/// event ID is resolved, matching the behavior of an empty context value.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EventIdOverride(Option<Arc<str>>);

impl EventIdOverride {
    /// Creates an override for downstream progress events.
    #[must_use]
    pub fn new(event_id: impl Into<Arc<str>>) -> Self {
        Self(Some(event_id.into()))
    }

    /// Returns the non-empty override, when one is set.
    #[must_use]
    pub fn get(&self) -> Option<&str> {
        self.0.as_deref().filter(|event_id| !event_id.is_empty())
    }

    fn resolve(&self, default: impl FnOnce() -> String) -> String {
        self.get().map_or_else(default, str::to_owned)
    }
}

/// Creates an event-ID override for downstream client operations.
#[must_use]
pub fn with_event_id(event_id: impl Into<Arc<str>>) -> EventIdOverride {
    EventIdOverride::new(event_id)
}

/// Formats an event ID for an operation on an existing container.
///
/// Container identifiers use Docker's familiar shorthand: a prefix through
/// the first colon is discarded, then at most the next 12 bytes are shown.
#[must_use]
pub fn container_event_id(
    event_id: &EventIdOverride,
    service_name: &str,
    container_id: &str,
    machine_name: &str,
) -> String {
    event_id.resolve(|| {
        format!(
            "{}{service_name}{}{container_id}{}{machine_name}",
            FAINT.render("Container "),
            FAINT.render("/"),
            FAINT.render(" on "),
            container_id = truncate_container_id(container_id),
        )
    })
}

/// Formats an event ID for a container whose Docker ID is not yet known.
#[must_use]
pub fn new_container_event_id(
    event_id: &EventIdOverride,
    container_name: &str,
    machine_name: &str,
) -> String {
    event_id.resolve(|| {
        format!(
            "{}{container_name}{}{machine_name}",
            FAINT.render("Container "),
            FAINT.render(" on "),
        )
    })
}

/// Formats an event ID for a pre-deploy hook operation.
#[must_use]
pub fn pre_deploy_hook_event_id(service_name: &str, machine_name: &str) -> String {
    format!(
        "{}{service_name}{}{machine_name}",
        FAINT.render("Pre-deploy hook "),
        FAINT.render(" on "),
    )
}

/// Formats an event ID for removing an old pre-deploy hook container.
#[must_use]
pub fn old_pre_deploy_hook_event_id(
    service_name: &str,
    container_id: &str,
    machine_name: &str,
) -> String {
    format!(
        "{}{service_name}{}{container_id}{}{machine_name}",
        FAINT.render("Old pre-deploy hook "),
        FAINT.render("/"),
        FAINT.render(" on "),
        container_id = truncate_container_id(container_id),
    )
}

/// Formats an event ID for pulling an image on a machine.
#[must_use]
pub fn image_event_id(image: &str, machine_name: &str) -> String {
    format!(
        "{}{image}{}{machine_name}",
        FAINT.render("Image "),
        FAINT.render(" on "),
    )
}

/// Formats an event ID for a volume operation on a machine.
#[must_use]
pub fn volume_event_id(volume_name: &str, machine_name: &str) -> String {
    format!(
        "{}{volume_name}{}{machine_name}",
        FAINT.render("Volume "),
        FAINT.render(" on "),
    )
}

/// Formats an event ID for a machine operation, including its public IP.
#[must_use]
pub fn machine_event_id(machine_name: &str, public_ip: &str) -> String {
    format!(
        "{}{machine_name}{}{public_ip}{}",
        FAINT.render("Machine "),
        FAINT.render(" ("),
        FAINT.render(")"),
    )
}

fn truncate_container_id(container_id: &str) -> &str {
    let identifier = container_id
        .split_once(':')
        .map_or(container_id, |(_, identifier)| identifier);
    let end = identifier
        .char_indices()
        .map(|(index, _)| index)
        .take_while(|index| *index <= 12)
        .last()
        .unwrap_or(0);

    if identifier.len() <= 12 {
        identifier
    } else if identifier.is_char_boundary(12) {
        &identifier[..12]
    } else {
        &identifier[..end]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: EventIdOverride = EventIdOverride(None);

    #[test]
    fn existing_container_uses_canonical_label_and_docker_short_id() {
        assert_eq!(
            container_event_id(&NONE, "web", "sha256:0123456789abcdef", "worker-1"),
            faint("Container ")
                + "web"
                + &faint("/")
                + "0123456789ab"
                + &faint(" on ")
                + "worker-1"
        );
    }

    #[test]
    fn docker_short_id_discards_only_the_first_prefix_and_keeps_short_ids() {
        assert_eq!(truncate_container_id("sha256:abc"), "abc");
        assert_eq!(truncate_container_id("abc"), "abc");
        assert_eq!(
            truncate_container_id("kind:abc:defghijklmnop"),
            "abc:defghijk"
        );
        assert_eq!(truncate_container_id(""), "");
    }

    #[test]
    fn short_id_never_returns_invalid_utf8_for_non_docker_input() {
        assert_eq!(truncate_container_id("12345678901界rest"), "12345678901");
        assert_eq!(truncate_container_id("123456789012界rest"), "123456789012");
    }

    #[test]
    fn non_empty_override_replaces_both_container_defaults_exactly() {
        let event_id = with_event_id(Arc::<str>::from("shared deploy line"));

        assert_eq!(
            container_event_id(&event_id, "ignored", "ignored", "ignored"),
            "shared deploy line"
        );
        assert_eq!(
            new_container_event_id(&event_id, "ignored", "ignored"),
            "shared deploy line"
        );
        assert_eq!(event_id.get(), Some("shared deploy line"));
    }

    #[test]
    fn empty_override_falls_back_to_the_generated_event_id() {
        let event_id = with_event_id("");

        assert_eq!(event_id.get(), None);
        assert_eq!(
            new_container_event_id(&event_id, "web-abc", "worker-1"),
            faint("Container ") + "web-abc" + &faint(" on ") + "worker-1"
        );
    }

    #[test]
    fn formats_new_container_and_pre_deploy_hook_labels() {
        assert_eq!(
            new_container_event_id(&NONE, "web-abc", "worker-1"),
            faint("Container ") + "web-abc" + &faint(" on ") + "worker-1"
        );
        assert_eq!(
            pre_deploy_hook_event_id("web", "worker-1"),
            faint("Pre-deploy hook ") + "web" + &faint(" on ") + "worker-1"
        );
        assert_eq!(
            old_pre_deploy_hook_event_id("web", "0123456789abcdef", "worker-1"),
            faint("Old pre-deploy hook ")
                + "web"
                + &faint("/")
                + "0123456789ab"
                + &faint(" on ")
                + "worker-1"
        );
    }

    #[test]
    fn formats_image_volume_and_machine_labels() {
        assert_eq!(
            image_event_id("registry.example/web:latest", "worker-1"),
            faint("Image ") + "registry.example/web:latest" + &faint(" on ") + "worker-1"
        );
        assert_eq!(
            volume_event_id("data", "worker-1"),
            faint("Volume ") + "data" + &faint(" on ") + "worker-1"
        );
        assert_eq!(
            machine_event_id("worker-1", "2001:db8::1"),
            faint("Machine ") + "worker-1" + &faint(" (") + "2001:db8::1" + &faint(")")
        );
    }

    #[test]
    fn labels_preserve_empty_dynamic_values_and_style_only_static_fragments() {
        assert_eq!(
            container_event_id(&NONE, "", "", ""),
            faint("Container ") + &faint("/") + &faint(" on ")
        );
        assert_eq!(
            machine_event_id("", ""),
            faint("Machine ") + &faint(" (") + &faint(")")
        );
    }

    fn faint(value: &str) -> String {
        format!("\x1b[2m{value}\x1b[0m")
    }
}
