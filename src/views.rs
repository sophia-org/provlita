//! Retained Xilem roots and immutable rendered target metadata. Protocol
//! authority is checked by the service before dispatching a toolkit callback.
use crate::{
    config::Config,
    ui::{DockHost, DockScene, TileIntent},
};
use masonry::{core::WidgetId, widgets::ButtonPress};
use sophia_protocol::{
    ContentAllocationId, ContentGrant, ContentOutputId, ContentPixelRect, ShellPersistentCatalog,
};
use std::collections::BTreeMap;
use xilem_masonry::core::DynMessage;

/// Exact acknowledged identity of a retained output host.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ViewIdentity {
    /// Connection/content authority.
    pub grant: ContentGrant,
    /// Current logical output identity.
    pub output: ContentOutputId,
    /// Acknowledged edge allocation.
    pub allocation: ContentAllocationId,
    /// Negotiated scale generation.
    pub scale_generation: u64,
}
type Key = (u64, u64, u64, u64, u64, u64, u64);
impl ViewIdentity {
    fn key(self) -> Key {
        (
            self.grant.connection_epoch,
            self.grant.content_grant_epoch,
            self.output.id,
            self.output.generation,
            self.allocation.id,
            self.allocation.generation,
            self.scale_generation,
        )
    }
    fn valid(self) -> bool {
        let key = self.key();
        [key.0, key.1, key.2, key.3, key.4, key.5, key.6]
            .iter()
            .all(|v| *v != 0)
    }
}
/// A target from the exact layout that produced the rendered scene.
#[derive(Clone, Debug)]
pub struct ViewTarget {
    /// Stable configured pin ordinal, never a catalog slot masquerading as identity.
    pub id: u64,
    /// Catalog slot captured in this view.
    pub slot: u16,
    /// Physical hit rectangle inside the transparent allocation raster.
    pub rect: ContentPixelRect,
    widget: WidgetId,
}
/// Lease-free origin carried with the GPU job and eventual candidate.
#[derive(Clone, Debug)]
pub struct ViewMetadata {
    /// Exact host identity.
    pub identity: ViewIdentity,
    /// Complete catalog revision used for this scene.
    pub catalog_generation: u64,
    /// Same-scene available targets; unavailable tiles have no target.
    pub targets: Vec<ViewTarget>,
}
/// Scene plus its immutable same-layout metadata; no protocol admission implied.
pub struct PreparedView {
    /// Bounded drawing commands transferred to the GPU worker.
    pub scene: Box<DockScene>,
    /// Origin retained by the service through actual presentation.
    pub metadata: ViewMetadata,
}
struct Host {
    ui: DockHost,
    width: u32,
    height: u32,
    scale: u64,
    catalog_generation: u64,
}
/// At most sixteen retained roots. The service holds this owner, so GPU waits
/// cannot block toolkit action dispatch. No secondary application reducer.
pub struct DockViews {
    config: Config,
    catalog: Option<ShellPersistentCatalog>,
    hosts: BTreeMap<Key, Host>,
}
impl DockViews {
    /// Construct without a display or GPU.
    pub fn new(config: Config) -> Self {
        Self {
            config,
            catalog: None,
            hosts: BTreeMap::new(),
        }
    }
    /// Install a complete decoded catalog. Validation precedes root mutation.
    pub fn install_catalog(&mut self, catalog: ShellPersistentCatalog) -> Result<(), String> {
        let wire = &catalog.catalog;
        if wire.connection_epoch == 0
            || wire.generation == 0
            || wire.entries.len() > 4096
            || catalog.identities.len() != wire.entries.len()
            || self.catalog.as_ref().is_some_and(|old| {
                old.catalog.connection_epoch != wire.connection_epoch
                    || old.catalog.generation >= wire.generation
            })
        {
            return Err("stale or invalid view catalog".into());
        }
        let entries = catalog_entries(&catalog)?;
        // The same UI validation runs before any of the retained roots changes.
        let mut validation = crate::ui::DockState::new(&self.config);
        validation.install_catalog(wire.generation, &entries)?;
        for host in self.hosts.values_mut() {
            host.ui.install_catalog(wire.generation, &entries)?;
            host.catalog_generation = wire.generation;
        }
        self.catalog = Some(catalog);
        Ok(())
    }
    /// Reconcile the existing root for this exact allocation and produce one
    /// centered scene. Geometry changes require a new identity.
    pub fn prepare(
        &mut self,
        identity: ViewIdentity,
        width: u32,
        height: u32,
        scale: f64,
    ) -> Result<PreparedView, String> {
        let catalog = self.catalog.as_ref().ok_or("no complete dock catalog")?;
        if !identity.valid() || identity.grant.connection_epoch != catalog.catalog.connection_epoch
        {
            return Err("view identity does not match catalog connection".into());
        }
        let key = identity.key();
        if let Some(host) = self.hosts.get(&key)
            && (host.width != width || host.height != height || host.scale != scale.to_bits())
        {
            return Err("allocation geometry changed without a new identity".into());
        }
        if !self.hosts.contains_key(&key) {
            // Construct and validate before removing an old root.
            let mut ui = DockHost::with_scale(&self.config, scale)?;
            ui.install_catalog(catalog.catalog.generation, &catalog_entries(catalog)?)?;
            ui.allocated_scene(width, height)?;
            let replaced = self
                .hosts
                .keys()
                .filter(|old| old.0 != key.0 || old.1 != key.1 || old.2 == key.2)
                .count();
            if self.hosts.len() - replaced >= 16 {
                return Err("dock retained host limit exhausted".into());
            }
            self.hosts
                .retain(|old, _| old.0 == key.0 && old.1 == key.1 && old.2 != key.2);
            self.hosts.insert(
                key,
                Host {
                    ui,
                    width,
                    height,
                    scale: scale.to_bits(),
                    catalog_generation: catalog.catalog.generation,
                },
            );
        }
        let host = self.hosts.get_mut(&key).expect("owned retained host");
        let (scene, layout) = host.ui.allocated_scene(width, height)?;
        if layout.len() != host.ui.state().tiles().len() {
            return Err("dock layout omitted a tile".into());
        }
        let mut targets = Vec::new();
        for (index, ((widget, rect), tile)) in
            layout.iter().zip(host.ui.state().tiles()).enumerate()
        {
            let Some(slot) = tile.slot else {
                continue;
            };
            let (x0, y0, x1, y1) = (
                rect.x0.floor(),
                rect.y0.floor(),
                rect.x1.ceil(),
                rect.y1.ceil(),
            );
            if x0 < 0.0
                || y0 < 0.0
                || x1 > f64::from(width)
                || y1 > f64::from(height)
                || x1 <= x0
                || y1 <= y0
            {
                return Err("dock target escapes allocated raster".into());
            }
            targets.push(ViewTarget {
                id: index as u64 + 1,
                slot,
                widget: *widget,
                rect: ContentPixelRect {
                    x: x0 as i32,
                    y: y0 as i32,
                    width: (x1 - x0) as u32,
                    height: (y1 - y0) as u32,
                },
            });
        }
        Ok(PreparedView {
            scene: Box::new(scene),
            metadata: ViewMetadata {
                identity,
                catalog_generation: host.catalog_generation,
                targets,
            },
        })
    }
    /// Dispatch after the protocol lifecycle authorized this exact presented
    /// target. The returned intent still requires atomic outbound admission.
    pub fn activate(&mut self, view: &ViewMetadata, target: u64) -> Option<TileIntent> {
        let host = self.hosts.get_mut(&view.identity.key())?;
        if host.catalog_generation != view.catalog_generation {
            return None;
        }
        let target = view.targets.iter().find(|entry| entry.id == target)?;
        let tile = host
            .ui
            .state()
            .tiles()
            .get(usize::try_from(target.id).ok()?.checked_sub(1)?)?;
        if tile.slot != Some(target.slot) {
            return None;
        }
        if host.ui.tile_layout().get((target.id - 1) as usize)?.0 != target.widget {
            return None;
        }
        if !host.ui.dispatch_widget_action(
            target.widget,
            DynMessage(Box::new(ButtonPress { button: None })),
        ) {
            return None;
        }
        host.ui.take_intent()
    }
}
fn catalog_entries(catalog: &ShellPersistentCatalog) -> Result<Vec<(String, u16, bool)>, String> {
    catalog
        .catalog
        .entries
        .iter()
        .map(|entry| {
            Ok((
                catalog
                    .identities
                    .get(&entry.slot)
                    .ok_or("catalog identity missing")?
                    .clone(),
                entry.slot,
                entry.available,
            ))
        })
        .collect()
}
