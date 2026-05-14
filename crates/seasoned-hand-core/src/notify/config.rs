//! `config/notify.toml` parser — feeds the
//! [`crate::notify::worker::NotifyWorker`] per-trigger routing table
//! and per-channel default target lookup.
//!
//! Format (architecture §2.7):
//!
//! ```toml
//! [trigger.task_finished]
//! channels = ["ntfy", "email"]
//!
//! [trigger.task_failed]
//! channels = ["email"]
//!
//! [trigger.briefing_pending]
//! channels = []  # opt-in: leave empty to disable
//!
//! [trigger.verifier_fail]
//! channels = ["ntfy"]
//!
//! [channel.ntfy]
//! default_target = "alerts"      # the ntfy topic
//! default_metadata = { priority = "default" }
//!
//! [channel.email]
//! default_target = "msgid:<owner@example.com>"
//! ```
//!
//! Unknown trigger names are ignored with a `notify_config_unknown_trigger`
//! warning — keeps the loader forgiving (mistyped trigger keys don't
//! break boot). Unknown channel names referenced in `channels = [...]`
//! also log + survive; the worker filters them out at dispatch time
//! via the registry lookup.
//!
//! refs: /specs/phase-2/architecture.md §2.7
//! refs: /specs/phase-2/stories/story-2.12.md

use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;
use serde_json::Value;
use thiserror::Error;

use super::worker::TargetResolver;
use crate::channel::NotifyTarget;

/// Trigger kinds the Phase 2 listener watches (architecture §2.7).
/// Anything outside this set in the config file is logged + ignored.
pub const KNOWN_TRIGGERS: &[&str] = &[
    "task_finished",
    "task_failed",
    "briefing_pending",
    "verifier_fail",
];

#[derive(Debug, Clone, Default, Deserialize)]
struct RawConfig {
    #[serde(default)]
    trigger: HashMap<String, TriggerSection>,
    #[serde(default)]
    channel: HashMap<String, ChannelSection>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct TriggerSection {
    #[serde(default)]
    channels: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct ChannelSection {
    #[serde(default)]
    default_target: Option<String>,
    #[serde(default)]
    default_metadata: Option<toml::Value>,
}

/// Resolved notify routing config. Operator config + sensible defaults
/// for the per-trigger channel routing AND the per-channel default
/// target (so a `NotifyRequest` without `target_override` resolves).
#[derive(Debug, Clone, Default)]
pub struct NotifyConfig {
    /// trigger_kind → list of channel names to fan out to.
    pub triggers: HashMap<String, Vec<String>>,
    /// channel_name → default [`NotifyTarget`].
    pub channels: HashMap<String, NotifyTarget>,
}

#[derive(Debug, Error)]
pub enum NotifyConfigError {
    #[error("read config: {0}")]
    Read(String),
    #[error("parse toml: {0}")]
    Parse(String),
}

impl NotifyConfig {
    /// Build an empty config (no triggers, no channels). Production
    /// boot calls [`Self::from_path`]; tests sometimes prefer to build
    /// inline via [`Self::insert_trigger`] + [`Self::insert_channel`].
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, NotifyConfigError> {
        let path = path.as_ref();
        let raw = std::fs::read_to_string(path)
            .map_err(|e| NotifyConfigError::Read(format!("{}: {e}", path.display())))?;
        Self::parse(&raw)
    }

    pub fn parse(raw: &str) -> Result<Self, NotifyConfigError> {
        let cfg: RawConfig =
            toml::from_str(raw).map_err(|e| NotifyConfigError::Parse(e.to_string()))?;
        Ok(cfg.into_resolved())
    }

    /// Append a trigger → channels entry. Existing entries are
    /// replaced. Used by tests to build a config without TOML.
    pub fn insert_trigger(&mut self, trigger: impl Into<String>, channels: Vec<String>) {
        self.triggers.insert(trigger.into(), channels);
    }

    /// Insert a per-channel default [`NotifyTarget`]. Tests use this
    /// to wire a static target without touching disk.
    pub fn insert_channel(&mut self, channel: impl Into<String>, target: NotifyTarget) {
        self.channels.insert(channel.into(), target);
    }

    /// Per-trigger channel list. Missing trigger → empty slice (the
    /// listener treats this as "trigger is configured off").
    pub fn channels_for(&self, trigger: &str) -> &[String] {
        match self.triggers.get(trigger) {
            Some(v) => v.as_slice(),
            None => &[],
        }
    }
}

impl RawConfig {
    fn into_resolved(self) -> NotifyConfig {
        let mut triggers: HashMap<String, Vec<String>> = HashMap::new();
        for (name, section) in self.trigger {
            if !KNOWN_TRIGGERS.contains(&name.as_str()) {
                tracing::warn!(
                    trigger = %name,
                    "notify_config_unknown_trigger; ignored — see KNOWN_TRIGGERS"
                );
                continue;
            }
            triggers.insert(name, section.channels);
        }
        let mut channels: HashMap<String, NotifyTarget> = HashMap::new();
        for (name, section) in self.channel {
            let Some(target_ref) = section.default_target else {
                continue;
            };
            let metadata = section
                .default_metadata
                .and_then(|v| toml_value_to_json(v).ok())
                .unwrap_or_else(|| Value::Object(Default::default()));
            channels.insert(
                name.clone(),
                NotifyTarget {
                    channel: name,
                    target_ref,
                    metadata,
                },
            );
        }
        NotifyConfig { triggers, channels }
    }
}

/// Bridge from `toml::Value` to `serde_json::Value` so the rest of
/// the kernel can carry one canonical JSON shape for metadata.
fn toml_value_to_json(value: toml::Value) -> Result<Value, NotifyConfigError> {
    let s = serde_json::to_string(&value).map_err(|e| NotifyConfigError::Parse(e.to_string()))?;
    serde_json::from_str(&s).map_err(|e| NotifyConfigError::Parse(e.to_string()))
}

impl TargetResolver for NotifyConfig {
    fn resolve(&self, channel: &str) -> Option<NotifyTarget> {
        self.channels.get(channel).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_triggers_and_channels() {
        let cfg = NotifyConfig::parse(
            r#"
            [trigger.task_finished]
            channels = ["ntfy", "email"]

            [trigger.task_failed]
            channels = ["email"]

            [channel.ntfy]
            default_target = "alerts"
            default_metadata = { priority = "high" }

            [channel.email]
            default_target = "msgid:<owner@example.com>"
            "#,
        )
        .expect("parse ok");

        assert_eq!(cfg.channels_for("task_finished"), &["ntfy", "email"]);
        assert_eq!(cfg.channels_for("task_failed"), &["email"]);
        assert!(cfg.channels_for("briefing_pending").is_empty());

        let ntfy = cfg.channels.get("ntfy").expect("ntfy default");
        assert_eq!(ntfy.target_ref, "alerts");
        assert_eq!(ntfy.metadata["priority"], "high");
    }

    #[test]
    fn unknown_trigger_is_ignored() {
        let cfg = NotifyConfig::parse(
            r#"
            [trigger.mystery_trigger]
            channels = ["ntfy"]

            [trigger.task_finished]
            channels = ["ntfy"]
            "#,
        )
        .expect("parse ok");
        assert!(cfg.channels_for("mystery_trigger").is_empty());
        assert_eq!(cfg.channels_for("task_finished"), &["ntfy"]);
    }

    #[test]
    fn empty_config_resolves_to_no_routing() {
        let cfg = NotifyConfig::empty();
        for trigger in KNOWN_TRIGGERS {
            assert!(cfg.channels_for(trigger).is_empty(), "trigger {trigger}");
        }
        assert!(cfg.resolve("ntfy").is_none());
    }
}
