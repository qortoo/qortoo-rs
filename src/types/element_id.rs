use std::fmt::{Debug, Display, Formatter};

use crate::types::timestamp::Timestamp;

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) struct ElementId {
    timestamp: Timestamp,
    delimiter: u32,
}

impl ElementId {
    pub(crate) fn new(timestamp: &Timestamp, delimiter: u32) -> Self {
        Self {
            timestamp: timestamp.clone(),
            delimiter,
        }
    }

    pub(crate) fn timestamp(&self) -> &Timestamp {
        &self.timestamp
    }

    pub(crate) fn delimiter(&self) -> u32 {
        self.delimiter
    }
}

impl Debug for ElementId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        Display::fmt(self, f)
    }
}

impl Display for ElementId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "[{}:{}:{}]",
            self.timestamp.lamport(),
            self.timestamp.cuid(),
            self.delimiter
        )
    }
}

#[cfg(test)]
mod tests_element_id {
    use std::collections::HashSet;

    use super::*;
    use crate::types::uid::Cuid;

    fn new_timestamp(lamport: u64, cuid: &str) -> Timestamp {
        Timestamp::new(lamport, &Cuid::try_from(cuid).unwrap())
    }

    #[test]
    fn can_create_an_element_id() {
        let timestamp = new_timestamp(1, "0000000000000001");
        let element_id = ElementId::new(&timestamp, 2);

        assert_eq!(element_id.timestamp(), &timestamp);
        assert_eq!(element_id.delimiter(), 2);
    }

    #[test]
    fn can_compare_exact_element_identity() {
        let timestamp = new_timestamp(1, "0000000000000001");
        let first = ElementId::new(&timestamp, 1);
        let second = ElementId::new(&timestamp, 2);

        assert_ne!(first, second);
    }

    #[test]
    fn can_hash_exact_element_identity() {
        let timestamp = new_timestamp(1, "0000000000000001");
        let first = ElementId::new(&timestamp, 1);
        let second = ElementId::new(&timestamp, 2);
        let element_ids = HashSet::from([first, second]);

        assert_eq!(element_ids.len(), 2);
    }

    #[test]
    fn can_order_element_ids_by_timestamp_then_delimiter() {
        let earlier = new_timestamp(1, "0000000000000001");
        let later = new_timestamp(2, "0000000000000001");

        assert!(ElementId::new(&earlier, u32::MAX) < ElementId::new(&later, 0));
        assert!(ElementId::new(&earlier, 1) < ElementId::new(&earlier, 2));
    }

    #[test]
    fn can_display_an_element_id() {
        let element_id = ElementId::new(&new_timestamp(1, "0000000000000001"), 2);

        assert_eq!(element_id.to_string(), "[1:0000000000000001:2]");
        assert_eq!(format!("{element_id:?}"), element_id.to_string());
    }
}
