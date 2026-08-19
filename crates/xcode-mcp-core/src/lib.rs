pub mod diagnostic;
pub mod error;

pub use diagnostic::{Diagnostic, DiagnosticSource, FixIt, FixItRange, ParseResult, Severity};
pub use error::{Error, Result};
