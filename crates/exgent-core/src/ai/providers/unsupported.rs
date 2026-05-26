use super::super::{ProviderAdapter, ProviderEvent, ProviderRequest};

#[derive(Clone, Debug)]
pub struct UnsupportedProvider {
    adapter: String,
}

impl UnsupportedProvider {
    pub(crate) fn new(adapter: impl Into<String>) -> Self {
        Self {
            adapter: adapter.into(),
        }
    }
}

impl ProviderAdapter for UnsupportedProvider {
    fn stream_events(&self, _request: ProviderRequest, emit: &mut dyn FnMut(ProviderEvent)) {
        emit(ProviderEvent::Error(format!(
            "unsupported adapter: {}",
            self.adapter
        )));
    }
}
