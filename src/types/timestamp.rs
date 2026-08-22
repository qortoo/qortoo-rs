use std::fmt::{Debug, Display, Formatter};

use crate::types::uid::Cuid;

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) struct Timestamp {
    lamport: u64,
    cuid: Cuid,
}

impl Timestamp {
    pub(crate) fn new(lamport: u64, cuid: &Cuid) -> Self {
        Self {
            lamport,
            cuid: cuid.clone(),
        }
    }

    pub(crate) fn lamport(&self) -> u64 {
        self.lamport
    }

    pub(crate) fn cuid(&self) -> &Cuid {
        &self.cuid
    }
}

impl Debug for Timestamp {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        Display::fmt(self, f)
    }
}

impl Display for Timestamp {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}:{}]", self.lamport, self.cuid)
    }
}

#[cfg(test)]
mod tests_timestamp {
    use std::collections::HashSet;

    use super::*;

    fn new_cuid(value: &str) -> Cuid {
        Cuid::try_from(value).unwrap()
    }

    #[test]
    fn can_create_a_timestamp() {
        let cuid = new_cuid("0000000000000001");
        let timestamp = Timestamp::new(1, &cuid);

        assert_eq!(timestamp.lamport(), 1);
        assert_eq!(timestamp.cuid(), &cuid);
    }

    #[test]
    fn can_compare_timestamp_identity() {
        let cuid = new_cuid("0000000000000001");
        let timestamp = Timestamp::new(1, &cuid);

        assert_eq!(timestamp, timestamp.clone());
        assert_ne!(timestamp, Timestamp::new(2, &cuid));
    }

    #[test]
    fn can_hash_timestamp_identity() {
        let cuid = new_cuid("0000000000000001");
        let timestamp = Timestamp::new(1, &cuid);
        let timestamps = HashSet::from([timestamp.clone(), timestamp]);

        assert_eq!(timestamps.len(), 1);
    }

    #[test]
    fn can_order_timestamps_by_lamport() {
        let cuid = new_cuid("0000000000000001");
        let earlier = Timestamp::new(1, &cuid);
        let later = Timestamp::new(2, &cuid);

        assert!(earlier < later);
        assert!(later > earlier);
    }

    #[test]
    fn can_break_timestamp_ordering_ties_by_cuid() {
        let lower = Timestamp::new(1, &new_cuid("0000000000000001"));
        let higher = Timestamp::new(1, &new_cuid("0000000000000002"));

        assert!(lower < higher);
        assert!(higher > lower);
    }

    #[test]
    fn can_compare_lamport_boundaries_without_overflow() {
        let cuid = new_cuid("0000000000000001");
        let oldest = Timestamp::new(0, &cuid);
        let newest = Timestamp::new(u64::MAX, &cuid);

        assert!(oldest < newest);
        assert!(newest > oldest);
    }

    #[test]
    fn can_display_a_timestamp() {
        let timestamp = Timestamp::new(1, &new_cuid("0000000000000001"));

        assert_eq!(timestamp.to_string(), "[1:0000000000000001]");
        assert_eq!(format!("{timestamp:?}"), timestamp.to_string());
    }
}
