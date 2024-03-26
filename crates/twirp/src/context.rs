use http::Extensions;

#[derive(Default)]
pub struct Context {
    pub extensions: Extensions,
    pub resp_extensions: Extensions,
}

impl Context {
    pub fn new(extensions: Extensions) -> Self {
        Self {
            extensions,
            resp_extensions: Extensions::new(),
        }
    }
}
