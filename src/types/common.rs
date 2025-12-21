use std::{fmt::Debug, sync::Arc};

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

pub type ArcStr = Arc<str>;

pub type ResourceID = String;
