use crate::connectivity::Connectivity;

pub struct NullConnectivity {}

impl NullConnectivity {
    pub fn new() -> Self {
        Self {}
    }
}

impl Connectivity for NullConnectivity {}
