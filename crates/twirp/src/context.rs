use std::sync::{Arc, Mutex};

use http::Extensions;

#[derive(Default)]
pub struct Context {
    extensions: Extensions,
    resp_extensions: Arc<Mutex<Extensions>>,
}

impl Context {
    pub fn new(extensions: Extensions, resp_extensions: Arc<Mutex<Extensions>>) -> Self {
        Self {
            extensions,
            resp_extensions,
        }
    }

    /// Get a request extension.
    pub fn get<T>(&self) -> Option<&T>
    where
        T: Clone + Send + Sync + 'static,
    {
        self.extensions.get::<T>()
    }

    // Insert a response extension.
    pub fn insert<T>(&self, val: T) -> Option<T>
    where
        T: Clone + Send + Sync + 'static,
    {
        self.resp_extensions
            .lock()
            .expect("mutex poisoned")
            .insert(val)
    }
}
