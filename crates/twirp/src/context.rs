use std::sync::Mutex;

use http::Extensions;

#[derive(Default)]
pub struct Context {
    pub extensions: Mutex<Extensions>,
}

impl Context {
    pub fn new(extensions: Extensions) -> Self {
        Self {
            extensions: Mutex::new(extensions),
        }
    }
}
