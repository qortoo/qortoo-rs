use std::fmt::{Debug, Display, Formatter};

use crate::{
    errors::datatypes::{DatatypeError, deserialize_error},
    types::uid::{Cuid, UID_LEN},
};

pub(crate) const TIMESTAMP_ENCODED_LEN: usize = size_of::<u64>() + UID_LEN;

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) struct Timestamp {
    lamport: u64,
    cuid: Cuid,
}

impl Timestamp {
    /// Returns the synthetic timestamp used by CRDT initial states.
    ///
    /// CRDTs that use this sentinel must reserve Lamport zero and require
    /// positive Lamport values for their real modification operations.
    pub(crate) fn initial() -> Self {
        Self {
            lamport: 0,
            cuid: Cuid::new_nil(),
        }
    }

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

    pub(crate) fn is_initial(&self) -> bool {
        self == &Self::initial()
    }

    pub(crate) fn to_bytes(&self) -> [u8; TIMESTAMP_ENCODED_LEN] {
        let mut encoded = [0; TIMESTAMP_ENCODED_LEN];
        encoded[..size_of::<u64>()].copy_from_slice(&self.lamport.to_le_bytes());
        encoded[size_of::<u64>()..].copy_from_slice(&self.cuid.to_bytes());
        encoded
    }

    pub(crate) fn from_bytes(encoded: &[u8]) -> Result<Self, DatatypeError> {
        if encoded.len() != TIMESTAMP_ENCODED_LEN {
            return Err(deserialize_error(
                "timestamp",
                format_args!(
                    "expected {TIMESTAMP_ENCODED_LEN} bytes, got {}",
                    encoded.len()
                ),
            ));
        }

        let mut lamport_bytes = [0; size_of::<u64>()];
        lamport_bytes.copy_from_slice(&encoded[..size_of::<u64>()]);
        let lamport = u64::from_le_bytes(lamport_bytes);
        let cuid = Cuid::from_bytes(&encoded[size_of::<u64>()..])?;
        Ok(Self { lamport, cuid })
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
    fn can_create_an_initial_timestamp_older_than_real_operations() {
        let initial = Timestamp::initial();
        let real = Timestamp::new(1, &new_cuid("----------------"));

        assert!(initial.is_initial());
        assert_eq!(initial.lamport(), 0);
        assert_eq!(initial.cuid(), &Cuid::new_nil());
        assert!(initial < real);
        assert!(!real.is_initial());
    }

    #[test]
    fn can_round_trip_timestamp_bytes() {
        let timestamps = [
            Timestamp::initial(),
            Timestamp::new(7, &new_cuid("0000000000000001")),
        ];

        for timestamp in timestamps {
            let encoded = timestamp.to_bytes();
            let decoded = Timestamp::from_bytes(&encoded).unwrap();

            assert_eq!(decoded, timestamp);
            assert_eq!(encoded.len(), TIMESTAMP_ENCODED_LEN);
            assert_eq!(
                &encoded[..size_of::<u64>()],
                &timestamp.lamport().to_le_bytes()
            );
            assert_eq!(&encoded[size_of::<u64>()..], &timestamp.cuid().to_bytes());
        }
    }

    #[test]
    fn can_reject_invalid_timestamp_bytes() {
        let error = Timestamp::from_bytes(&[]).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("deserialize: timestamp: expected 24 bytes, got 0")
        );

        let mut invalid_utf8 = Timestamp::initial().to_bytes();
        invalid_utf8[size_of::<u64>()] = 0xff;
        let error = Timestamp::from_bytes(&invalid_utf8).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("deserialize: uid: not valid UTF-8")
        );

        let mut invalid_format = Timestamp::initial().to_bytes();
        invalid_format[size_of::<u64>()] = b'(';
        let error = Timestamp::from_bytes(&invalid_format).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("deserialize: uid: invalid format")
        );
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
