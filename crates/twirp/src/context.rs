use http::Extensions;

#[derive(Default, Clone)]
pub struct Context {
    pub extensions: Extensions,
}

impl Context {
    pub fn new(extensions: Extensions) -> Self {
        Self { extensions }
    }
}
