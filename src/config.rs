//! KDL pin selection is configuration, never permission to launch commands.
use kdl::{KdlDocument, KdlNode};
use std::collections::BTreeSet;

/// Maximum accepted configuration size before parsing.
pub const MAX_CONFIG_BYTES: usize = 64 * 1024;
/// Maximum persistent tiles in the first dock profile.
pub const MAX_PINS: usize = 16;

/// Explicit local-output selection; the service binds IDs to current generations.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Outputs {
    /// Follow every currently admitted output.
    All,
    /// Follow only these logical output IDs.
    Selected(Vec<u64>),
}

/// A saved reference to an authorized catalog identity, not a command string.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Pin {
    /// Stable identity to resolve against the current catalog.
    pub identity: String,
    /// Operator-selected display label.
    pub label: String,
    /// Bundled vector symbol name; never a host filesystem path.
    pub icon: String,
}

/// First dock profile: bottom edge, centered alignment and bounded square tiles.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Config {
    /// Logical outputs to follow.
    pub outputs: Outputs,
    /// Tile edge length in logical pixels.
    pub tile_size: u16,
    /// Inter-tile spacing in logical pixels.
    pub spacing: u16,
    /// Ordered stable application references.
    pub pins: Vec<Pin>,
}

fn plain(node: &KdlNode, count: usize) -> Result<(), String> {
    if node.ty().is_some()
        || node.children().is_some()
        || node.entries().len() != count
        || node
            .entries()
            .iter()
            .any(|entry| entry.name().is_some() || entry.ty().is_some())
    {
        return Err(format!(
            "{} requires {count} untyped positional values",
            node.name().value()
        ));
    }
    Ok(())
}
fn number(node: &KdlNode, min: u16, max: u16) -> Result<u16, String> {
    plain(node, 1)?;
    node.get(0)
        .and_then(|v| v.as_integer())
        .and_then(|v| u16::try_from(v).ok())
        .filter(|v| (min..=max).contains(v))
        .ok_or_else(|| format!("{} must be in {min}..{max}", node.name().value()))
}
fn bounded_text(value: &str, limit: usize) -> bool {
    !value.is_empty() && value.len() <= limit && !value.chars().any(char::is_control)
}
fn pin(node: &KdlNode) -> Result<Pin, String> {
    if node.ty().is_some()
        || node.children().is_some()
        || node.entries().len() != 3
        || node.entries().iter().any(|v| v.ty().is_some())
    {
        return Err("pin requires an identity, label and bundled icon".into());
    }
    let identity = node
        .get(0)
        .and_then(|v| v.as_string())
        .filter(|v| {
            bounded_text(v, 256)
                && ["registered:", "desktop:"]
                    .iter()
                    .any(|prefix| v.strip_prefix(prefix).is_some_and(|tail| !tail.is_empty()))
        })
        .ok_or("invalid pin identity")?;
    let label = node
        .get("label")
        .and_then(|v| v.as_string())
        .filter(|v| bounded_text(v, 48))
        .ok_or("invalid pin label")?;
    let icon = node
        .get("icon")
        .and_then(|v| v.as_string())
        .filter(|v| matches!(*v, "terminal" | "browser" | "files"))
        .ok_or("icon must name terminal, browser or files")?;
    let positional = node.entries().iter().filter(|v| v.name().is_none()).count();
    let names: BTreeSet<_> = node
        .entries()
        .iter()
        .filter_map(|v| v.name().map(|n| n.value()))
        .collect();
    if positional != 1 || names != BTreeSet::from(["label", "icon"]) {
        return Err("unknown or repeated pin setting".into());
    }
    Ok(Pin {
        identity: identity.into(),
        label: label.into(),
        icon: icon.into(),
    })
}

impl Config {
    /// Parse bounded configuration. Catalog resolution is a later service step.
    pub fn parse(text: &str) -> Result<Self, String> {
        if text.len() > MAX_CONFIG_BYTES {
            return Err("dock configuration exceeds 64 KiB".into());
        }
        let document: KdlDocument = text.parse().map_err(|e| format!("invalid KDL: {e}"))?;
        if document.nodes().len() != 2 {
            return Err("expected schema and dock exactly once".into());
        }
        let schema = document.get("schema").ok_or("schema is required")?;
        if number(schema, 1, 1)? != 1 {
            return Err("unsupported schema".into());
        }
        let dock = document.get("dock").ok_or("dock is required")?;
        if dock.ty().is_some() || !dock.entries().is_empty() {
            return Err("dock takes a child block only".into());
        }
        let children = dock.children().ok_or("dock requires settings")?;
        let mut seen = BTreeSet::new();
        let mut identities = BTreeSet::new();
        let mut config = Self {
            outputs: Outputs::All,
            tile_size: 48,
            spacing: 6,
            pins: Vec::new(),
        };
        for node in children.nodes() {
            let name = node.name().value();
            if name != "pin" && !seen.insert(name) {
                return Err(format!("repeated {name}"));
            }
            match name {
                "outputs" => {
                    plain(node, node.entries().len())?;
                    if node.entries().len() == 1
                        && node.get(0).and_then(|v| v.as_string()) == Some("all")
                    {
                        config.outputs = Outputs::All;
                    } else {
                        if node.entries().is_empty() || node.entries().len() > 16 {
                            return Err("outputs requires 1..16 IDs or all".into());
                        }
                        let mut ids = Vec::new();
                        for value in node.entries() {
                            let id = value
                                .value()
                                .as_integer()
                                .and_then(|v| u64::try_from(v).ok())
                                .filter(|v| *v != 0)
                                .ok_or("invalid output ID")?;
                            if ids.contains(&id) {
                                return Err("duplicate output ID".into());
                            }
                            ids.push(id);
                        }
                        config.outputs = Outputs::Selected(ids);
                    }
                }
                "edge" | "align" => {
                    plain(node, 1)?;
                    let expected = if name == "edge" { "bottom" } else { "center" };
                    if node.get(0).and_then(|v| v.as_string()) != Some(expected) {
                        return Err(format!("{name} must be {expected}"));
                    }
                }
                "tile-size" => config.tile_size = number(node, 16, 128)?,
                "spacing" => config.spacing = number(node, 0, 32)?,
                "pin" => {
                    if config.pins.len() == MAX_PINS {
                        return Err("too many pins".into());
                    }
                    let pin = pin(node)?;
                    if !identities.insert(pin.identity.clone()) {
                        return Err("duplicate pin identity".into());
                    }
                    config.pins.push(pin);
                }
                _ => return Err(format!("unknown dock setting {name}")),
            }
        }
        if config.pins.is_empty() {
            return Err("dock requires at least one pin".into());
        }
        Ok(config)
    }
    /// Minimum logical strip dimensions including eight-pixel outer padding.
    pub fn logical_size(&self) -> (u32, u32) {
        let count = self.pins.len() as u32;
        (
            count * u32::from(self.tile_size)
                + count.saturating_sub(1) * u32::from(self.spacing)
                + 16,
            u32::from(self.tile_size) + 16,
        )
    }
}
