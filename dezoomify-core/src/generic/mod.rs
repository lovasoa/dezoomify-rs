//! Generic URL-template discovery backed by core's executable adaptive plan.

use std::sync::Arc;

use crate::core::adaptive::{GenericAdaptivePlan, is_generic_template};
use crate::core::discovery::DiscoveryEvent;
use crate::core::{
    CatalogEntry, Dezoomer, DezoomerMeta, DiscoveryDiagnostic, DiscoveryError, DiscoveryInput,
    DiscoveryStep, ImageCatalog, ImageDescriptor, LevelDescriptor, LevelPlan, StableId,
};

/// Generic template dezoomer.
pub struct Generic {
    template: String,
    complete: bool,
}

impl Dezoomer for Generic {
    fn advance(&mut self, event: DiscoveryEvent<'_>) -> Result<DiscoveryStep, DiscoveryError> {
        match event {
            DiscoveryEvent::Start if !is_generic_template(&self.template) => Ok(
                DiscoveryStep::Reject(DiscoveryDiagnostic::from("not a generic X/Y tile template")),
            ),
            DiscoveryEvent::Start if !self.complete => {
                self.complete = true;
                let level_id = StableId::new("generic:level");
                let plan = Arc::new(GenericAdaptivePlan::new(
                    level_id.clone(),
                    self.template.clone(),
                ));
                Ok(DiscoveryStep::Complete(ImageCatalog::new([
                    CatalogEntry::Ready(ImageDescriptor {
                        id: StableId::new("generic:image"),
                        title: Some(self.template.clone()),
                        format: StableId::new("generic"),
                        levels: vec![LevelDescriptor {
                            id: level_id,
                            plan: LevelPlan::Adaptive(plan),
                            ..Default::default()
                        }],
                        ..Default::default()
                    }),
                ])))
            }
            DiscoveryEvent::Resource(_) => Err(DiscoveryError::Session(
                "generic discovery requests no metadata".into(),
            )),
            DiscoveryEvent::Start => Err(DiscoveryError::Session(
                "generic session started twice".into(),
            )),
        }
    }
}

impl DezoomerMeta for Generic {
    const NAME: &'static str = "generic";
    const URL_HINTS: &'static [&'static str] = &["{{"];

    fn start(input: &DiscoveryInput) -> Self {
        Self {
            template: input.uri.clone(),
            complete: false,
        }
    }
}
