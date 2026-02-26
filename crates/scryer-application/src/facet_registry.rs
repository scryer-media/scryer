use std::collections::HashMap;
use std::sync::Arc;

use scryer_domain::MediaFacet;

use crate::facet_handler::FacetHandler;

pub struct FacetRegistry {
    handlers: HashMap<MediaFacet, Arc<dyn FacetHandler>>,
}

impl Default for FacetRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl FacetRegistry {
    pub fn new() -> Self {
        Self {
            handlers: HashMap::new(),
        }
    }

    pub fn register(&mut self, handler: Arc<dyn FacetHandler>) {
        self.handlers.insert(handler.facet(), handler);
    }

    pub fn get(&self, facet: &MediaFacet) -> Option<&Arc<dyn FacetHandler>> {
        self.handlers.get(facet)
    }

    pub fn all(&self) -> impl Iterator<Item = &Arc<dyn FacetHandler>> {
        self.handlers.values()
    }

    pub fn facet_ids(&self) -> Vec<&str> {
        self.handlers.values().map(|h| h.facet_id()).collect()
    }
}
