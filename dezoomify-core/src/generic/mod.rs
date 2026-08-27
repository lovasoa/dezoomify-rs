//! Generic URL-template discovery backed by core's executable adaptive plan.

use crate::core::adaptive::is_generic_template;
use crate::core::{
    CatalogEntry, DezoomerSpec, DiscoverableGrid, ImageCatalog, ImageDescriptor, LevelDescriptor,
    StableId,
};

pub const SPEC: DezoomerSpec = DezoomerSpec::immediate("generic", |template| Ok(catalog(template)))
    .recognizing(is_generic_template, "not a generic X/Y tile template")
    .preferring(|uri| uri.contains("{{"));

fn catalog(template: &str) -> ImageCatalog {
    ImageCatalog::new([CatalogEntry::Ready(ImageDescriptor {
        id: StableId::new("generic:image"),
        title: Some(template.to_owned()),
        format: StableId::new("generic"),
        levels: vec![LevelDescriptor::new(DiscoverableGrid::new(
            StableId::new("generic:level"),
            template.to_owned(),
        ))],
        ..Default::default()
    })])
}

#[test]
fn valid_template_completes_on_start_without_resources() {
    let mut registry = crate::core::Registry::new();
    registry.register(SPEC);
    let mut operation = registry.start("tiles/{{X}}/{{Y}}.jpg");
    assert!(operation.missing_resources().unwrap().is_empty());
    assert!(operation.is_complete());
    assert_eq!(operation.finish().unwrap().len(), 1);
}
