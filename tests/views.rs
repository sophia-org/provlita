//! Real retained roots and callback identity, without protocol or native authority.
use provlita::{
    config::Config,
    views::{DockViews, ViewIdentity},
};
use sophia_protocol::*;
use std::collections::BTreeMap;

fn catalog(generation: u64) -> ShellPersistentCatalog {
    ShellPersistentCatalog {
        catalog: ShellApplicationCatalog {
            connection_epoch: 1,
            generation,
            entries: vec![ShellApplicationDescriptor {
                slot: 2,
                available: true,
                label: "terminal".into(),
                keywords: String::new(),
            }],
        },
        identities: BTreeMap::from([(2, "registered:terminal".into())]),
    }
}
fn identity(output: u64) -> ViewIdentity {
    ViewIdentity {
        grant: ContentGrant {
            connection_epoch: 1,
            content_grant_epoch: 2,
        },
        output: ContentOutputId {
            id: output,
            generation: 1,
        },
        allocation: ContentAllocationId {
            id: output,
            generation: 1,
        },
        scale_generation: 1,
    }
}
fn views() -> DockViews {
    let mut views =
        DockViews::new(Config::parse(include_str!("../examples/minimal/config.kdl")).unwrap());
    views.install_catalog(catalog(1)).unwrap();
    views
}
#[test]
fn real_retained_roots_are_independent_and_catalog_replacement_invalidates_old_callback() {
    let mut views = views();
    let a = views.prepare(identity(1), 800, 64, 1.0).unwrap();
    let b = views.prepare(identity(2), 1000, 64, 1.0).unwrap();
    assert_eq!(
        a.metadata.targets.len(),
        1,
        "unavailable pins have no input targets"
    );
    assert!(b.metadata.targets[0].rect.x > a.metadata.targets[0].rect.x);
    assert_eq!(views.activate(&a.metadata, 1).unwrap().slot, 2);
    assert_eq!(
        views.activate(&b.metadata, 1).unwrap().identity,
        "registered:terminal"
    );
    let mut replacement = identity(1);
    replacement.allocation.generation = 2;
    let next = views.prepare(replacement, 800, 64, 1.0).unwrap();
    assert!(
        views.activate(&a.metadata, 1).is_none(),
        "old exact root retired"
    );
    assert!(views.activate(&next.metadata, 1).is_some());
    assert!(
        views.activate(&b.metadata, 1).is_some(),
        "neighbor widget survives replacement"
    );
    views.install_catalog(catalog(2)).unwrap();
    assert!(views.activate(&next.metadata, 1).is_none());
    let current = views.prepare(replacement, 800, 64, 1.0).unwrap();
    assert_eq!(
        views
            .activate(&current.metadata, 1)
            .unwrap()
            .catalog_generation,
        2
    );
}
#[test]
fn refused_geometry_catalog_and_identity_preserve_existing_view() {
    let mut views = views();
    let view = views.prepare(identity(1), 800, 64, 1.0).unwrap();
    assert!(views.prepare(identity(1), 801, 64, 1.0).is_err());
    let mut wrong = identity(1);
    wrong.grant.connection_epoch = 3;
    assert!(views.prepare(wrong, 800, 64, 1.0).is_err());
    let mut bad = catalog(2);
    bad.identities.clear();
    assert!(views.install_catalog(bad).is_err());
    assert_eq!(
        views
            .activate(&view.metadata, 1)
            .unwrap()
            .catalog_generation,
        1
    );
    assert!(views.activate(&view.metadata, 999).is_none());
}
