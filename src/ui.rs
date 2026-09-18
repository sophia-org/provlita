//! Xilem application state and retained views. No secondary reducer/event loop.
//! A widget intent is not a Sophia input event or permission to execute an app.
use crate::config::Config;
use masonry::{layout::AsUnit, peniko::Color};
use xilem_masonry::{
    AnyWidgetView, WidgetView,
    style::Style as _,
    view::{button, canvas, flex_col, flex_row, label},
};

mod host;
mod icons;
pub use host::{DockHost, DockScene};

/// Semantic tile data supplied by the catalog adapter, without command strings.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Tile {
    /// Stable configured catalog identity, independent of label or slot.
    pub identity: String,
    /// User-visible label from configuration.
    pub label: String,
    /// Bundled symbol selection.
    pub icon: String,
    /// Current authorized catalog slot; unavailable entries have none.
    pub slot: Option<u16>,
}

/// Bounded request for an adapter to resolve against exact presented authority.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TileIntent {
    /// Stable identity selected by the view callback.
    pub identity: String,
    /// Catalog generation captured by that callback's view.
    pub catalog_generation: u64,
    /// Slot captured by that view, not a command or transient opening.
    pub slot: u16,
}

/// One Xilem-owned semantic state; widget toolkit state remains in Masonry.
pub struct DockState {
    tiles: Vec<Tile>,
    catalog_generation: u64,
    tile_size: u16,
    spacing: u16,
    intent: Option<TileIntent>,
}
impl DockState {
    /// Initialize configured pins as unavailable until a complete catalog arrives.
    pub fn new(config: &Config) -> Self {
        Self {
            tiles: config
                .pins
                .iter()
                .map(|pin| Tile {
                    identity: pin.identity.clone(),
                    label: pin.label.clone(),
                    icon: pin.icon.clone(),
                    slot: None,
                })
                .collect(),
            catalog_generation: 0,
            tile_size: config.tile_size,
            spacing: config.spacing,
            intent: None,
        }
    }
    /// Current immutable tile semantics; these contain no launch authority.
    pub fn tiles(&self) -> &[Tile] {
        &self.tiles
    }

    /// Install one already-decoded complete catalog, resolving by identity only.
    /// Reject conflicting slot/name assignments before changing any state.
    pub fn install_catalog(
        &mut self,
        generation: u64,
        entries: &[(String, u16, bool)],
    ) -> Result<(), &'static str> {
        if generation == 0 || generation <= self.catalog_generation || entries.len() > 4096 {
            return Err("stale or oversized catalog");
        }
        let mut names = std::collections::BTreeSet::new();
        let mut slots = std::collections::BTreeSet::new();
        for (name, slot, _) in entries {
            if name.len() > 256
                || name.is_empty()
                || *slot == 0
                || *slot > 4096
                || !names.insert(name)
                || !slots.insert(slot)
            {
                return Err("invalid catalog identity mapping");
            }
        }
        for tile in &mut self.tiles {
            tile.slot = entries
                .iter()
                .find(|(name, _, _)| name == &tile.identity)
                .and_then(|(_, slot, available)| available.then_some(*slot));
        }
        self.catalog_generation = generation;
        // A queued old intent is not retargeted to new slots. Its exact captured
        // generation remains available for the adapter to reject as stale.
        Ok(())
    }
    /// Transfer one callback intent. The protocol adapter still must join it to
    /// a real exact Presented/Action; this method never fabricates that event.
    pub fn take_intent(&mut self) -> Option<TileIntent> {
        self.intent.take()
    }
}

/// Construct the dock view. Xilem callbacks mutate this application's state
/// directly; no second application dispatcher or reducer sits between them.
pub fn dock_view(state: &DockState) -> impl WidgetView<DockState> + use<> {
    let tiles: Vec<Box<AnyWidgetView<DockState>>> = state
        .tiles
        .iter()
        .map(|tile| {
            let identity = tile.identity.clone();
            let generation = state.catalog_generation;
            let slot = tile.slot;
            let icon = tile.icon.clone();
            let picture = canvas(move |_state: &mut DockState, _ctx, scene, size| {
                icons::draw(scene, size, &icon, slot.is_some());
            })
            .alt_text(tile.label.clone())
            .dims((30.px(), 28.px()));
            let text = label(tile.label.clone())
                .text_size(9.0)
                .line_break_mode(masonry::properties::LineBreaking::Clip)
                .width(f64::from(state.tile_size.saturating_sub(2)).px())
                .color(if slot.is_some() {
                    Color::from_rgb8(235, 235, 235)
                } else {
                    Color::from_rgb8(115, 115, 115)
                });
            Box::new(
                button(
                    flex_col((picture, text)).gap(0.px()),
                    move |state: &mut DockState| {
                        if state.intent.is_none()
                            && state.catalog_generation == generation
                            && state
                                .tiles
                                .iter()
                                .any(|tile| tile.identity == identity && tile.slot == slot)
                            && let Some(slot) = slot
                        {
                            state.intent = Some(TileIntent {
                                identity: identity.clone(),
                                catalog_generation: generation,
                                slot,
                            });
                        }
                    },
                )
                .dims((
                    f64::from(state.tile_size).px(),
                    f64::from(state.tile_size).px(),
                ))
                .padding(0.px())
                .corner_radius(0.px())
                .border_width(1.px())
                .border_color(Color::from_rgb8(174, 180, 189))
                .background_color(if slot.is_some() {
                    Color::from_rgb8(55, 62, 73)
                } else {
                    Color::from_rgb8(41, 44, 49)
                }),
            ) as Box<AnyWidgetView<DockState>>
        })
        .collect();
    flex_row(tiles)
        .gap(f64::from(state.spacing).px())
        .padding(8.px())
}
