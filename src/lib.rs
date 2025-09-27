use std::fmt::Debug;

pub use datatypes::datatype_set::DatatypeSet;

pub use crate::{
    clients::client::Client,
    datatypes::{builder::DatatypeBuilder, counter::Counter, datatype::Datatype},
    errors::{clients::ClientError, datatypes::DatatypeError},
    types::datatype::{DataType, DatatypeState},
};

pub(crate) mod clients;
mod constants;
pub(crate) mod datatypes;
pub(crate) mod defaults;
pub(crate) mod errors;
pub(crate) mod observability;
pub(crate) mod operations;
pub(crate) mod types;
pub(crate) mod utils;

/// A trait for types that can be converted into a String and debugged.
///
/// This trait combines `Into<String>` and `Debug` bounds for convenience
/// in function parameters that need both string conversion and debug output.
///
/// # Note
///
/// This trait is automatically implemented for all types that satisfy
/// both `Into<String>` and `Debug`
pub trait IntoString: Into<String> + Debug {}

impl<T: Into<String> + Debug> IntoString for T {}

#[cfg(feature = "tracing")]
#[ctor::ctor]
pub fn init_tracing_subscriber() {
    use tracing::level_filters::LevelFilter;
    observability::tracing_for_test::init(LevelFilter::TRACE);
}
