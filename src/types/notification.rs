use derive_more::Display;

use crate::types::uid::{Cuid, Duid};

#[derive(Debug, Clone, Display)]
#[display("Notification{{ cuid:{cuid}, duid:{duid}, sseq:{sseq}, safe:{safe} }}")]
pub struct Notification {
    pub cuid: Cuid,
    pub duid: Duid,
    pub sseq: u64,
    pub safe: u64,
}

impl Notification {
    pub fn new(cuid: Cuid, duid: Duid, sseq: u64, safe: u64) -> Self {
        Self {
            cuid,
            duid,
            sseq,
            safe,
        }
    }
}

#[cfg(test)]
mod tests_notification {
    use tracing::info;

    use crate::types::{
        notification::Notification,
        uid::{Cuid, Duid},
    };

    #[test]
    fn can_display() {
        let notification = Notification::new(Cuid::new_nil(), Duid::new_nil(), 1, 1);
        info!("{}", notification);
    }
}
