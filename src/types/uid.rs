use std::{
    fmt::{Debug, Display, Formatter},
    hash::{Hash, Hasher},
    ops::Deref,
    sync::Arc,
};

use nanoid::nanoid;

use crate::types::common::ArcStr;

pub type Cuid = Uid;
pub type Duid = Uid;

pub const UID_LEN: usize = 16;

#[derive(Clone, PartialEq, PartialOrd, Eq, Ord)]
pub struct Uid(ArcStr);

impl Uid {
    pub fn new() -> Self {
        Self(nanoid!(UID_LEN).into())
    }

    pub fn new_nil() -> Self {
        Self(Arc::from("0000000000000000"))
    }

    fn validate(s: &str) -> bool {
        s.len() == UID_LEN
            && s.chars()
                .all(|c| c == '-' || c == '_' || c.is_ascii_alphanumeric())
    }
}

impl Debug for Uid {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Uid").field(&self.0).finish()
    }
}

impl Default for Uid {
    fn default() -> Self {
        Self::new_nil()
    }
}

impl Display for Uid {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Hash for Uid {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

impl TryFrom<String> for Uid {
    type Error = &'static str;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        if Self::validate(&value) {
            Ok(Self(value.into()))
        } else {
            Err("Invalid UID format")
        }
    }
}

impl TryFrom<&str> for Uid {
    type Error = &'static str;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        if Self::validate(value) {
            Ok(Self(Arc::from(value)))
        } else {
            Err("Invalid UID format")
        }
    }
}

impl AsRef<str> for Uid {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl Deref for Uid {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.0.deref()
    }
}

#[cfg(test)]
mod tests_uid {
    use std::collections::HashSet;

    use rstest::rstest;
    use tracing::info;

    use super::*;

    #[test]
    fn can_create_duid_and_cuid() {
        let _duid = Duid::new();
        let _cuid = Cuid::new();
        assert_ne!(_duid.to_string(), _cuid.to_string());
        let default_duid = Duid::default();
        info!("{default_duid}");
    }

    #[rstest]
    #[case::valid1("0000000000000000", true)]
    #[case::valid2("-_00000000000000", true)]
    #[case::invalid_characters("()00000000000000", false)]
    #[case::invalid_short("short", false)]
    #[case::invalid_long("longer_than_16_characters", false)]
    fn can_validate_uids(#[case] uid: &str, #[case] expected: bool) {
        assert_eq!(expected, Uid::validate(uid));
    }

    #[test]
    fn can_generate_uids() {
        let mut uid_set = HashSet::new();
        uid_set.insert(Cuid::new_nil());
        const LIMIT: usize = 10000;
        for _n in 0..LIMIT {
            let uid = Cuid::new();
            assert!(Cuid::validate(&uid.0));
            uid_set.insert(uid);
        }
        assert_eq!(uid_set.len(), LIMIT + 1)
    }

    #[rstest]
    #[case::valid1("0000000000000000", true)]
    #[case::valid2("-_00000000000000", true)]
    #[case::invalid_characters("()00000000000000", false)]
    #[case::invalid_short("short", false)]
    #[case::invalid_long("longer_than_16_characters", false)]
    fn can_try_from(#[case] uid: &str, #[case] expected: bool) {
        can_try_from_string(uid.to_string(), expected);
        can_try_from_str(uid, expected);
    }

    fn can_try_from_string(input: String, should_succeed: bool) {
        let result = Uid::try_from(input.clone());

        if should_succeed {
            assert!(result.is_ok());
            let uid = result.unwrap();
            assert_eq!(uid.to_string(), input);
        } else {
            assert!(result.is_err());
        }
    }

    fn can_try_from_str(input: &str, should_succeed: bool) {
        let result = Uid::try_from(input);

        if should_succeed {
            assert!(result.is_ok());
            let uid = result.unwrap();
            assert_eq!(uid.as_ref(), input);
        } else {
            assert!(result.is_err());
        }
    }
}
