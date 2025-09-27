#[derive(Debug)]
pub struct DatatypeOption {}

impl DatatypeOption {
    pub fn new() -> Self {
        Self {}
    }
}

impl Default for DatatypeOption {
    fn default() -> Self {
        Self::new()
    }
}
