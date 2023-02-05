//! Errors generated by the compiler.
use crate::{core::Id, frontend};
use std::collections::HashSet;
use std::rc::Rc;

/// A span of the input program.
/// Used for reporting location-based errors.
#[derive(Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct Span {
    /// Reference to input program source.
    pub input: Rc<str>,
    /// Name of the input file
    pub file: Rc<str>,
    /// The start of the span.
    pub start: usize,
    /// The end of the span.
    pub end: usize,
}

impl Span {
    /// Create a new `Error::Span` from a `pest::Span` and
    /// the input string.
    pub fn new(span: pest::Span, file: Rc<str>, input: Rc<str>) -> Span {
        Span {
            input,
            file,
            start: span.start(),
            end: span.end(),
        }
    }
}

/// An IR node that may contain position information.
pub trait WithPos {
    /// Add span information to this node
    fn set_span(self, sp: Option<Span>) -> Self;

    /// Copy the span associated with this node.
    fn copy_span(&self) -> Option<Span>;
}

pub struct Error {
    pub kind: Box<ErrorKind>,
    pub notes: Vec<(String, Option<Span>)>,
}

impl Error {
    pub fn files(&self) -> impl Iterator<Item = (Rc<str>, Rc<str>)> {
        let set: HashSet<(Rc<str>, Rc<str>)> = HashSet::from_iter(
            self.notes
                .iter()
                .filter_map(|(_, sp)| sp.as_ref())
                .map(|pos| (pos.file.clone(), pos.input.clone())),
        );
        set.into_iter()
    }
}

impl std::fmt::Debug for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.kind)
    }
}

impl Error {
    pub fn add_note<S: ToString>(mut self, msg: S, pos: Option<Span>) -> Self {
        self.notes.push((msg.to_string(), pos));
        self
    }

    pub fn parse_error(err: pest_consume::Error<frontend::Rule>) -> Self {
        Self {
            kind: Box::new(ErrorKind::ParseError(err)),
            notes: vec![],
        }
    }

    pub fn invalid_file(f: String) -> Self {
        Self {
            kind: Box::new(ErrorKind::InvalidFile(f)),
            notes: vec![],
        }
    }

    pub fn write_error(e: String) -> Self {
        Self {
            kind: Box::new(ErrorKind::WriteError(e)),
            notes: vec![],
        }
    }

    pub fn malformed<S: ToString>(msg: S) -> Self {
        Self {
            kind: Box::new(ErrorKind::Malformed(msg.to_string())),
            notes: vec![],
        }
    }

    pub fn undefined<S: ToString>(name: Id, kind: S) -> Self {
        Self {
            kind: Box::new(ErrorKind::Undefined(name, kind.to_string())),
            notes: vec![],
        }
    }

    pub fn already_bound<S: ToString>(name: Id, kind: S) -> Self {
        Self {
            kind: Box::new(ErrorKind::AlreadyBound(name, kind.to_string())),
            notes: vec![],
        }
    }

    pub fn misc(msg: String) -> Self {
        Self {
            kind: Box::new(ErrorKind::Misc(msg)),
            notes: vec![],
        }
    }
}

/// Standard error type for Calyx errors.
#[allow(clippy::large_enum_variant)]
pub enum ErrorKind {
    /// Error while parsing a Calyx program.
    ParseError(pest_consume::Error<frontend::Rule>),
    /// The input file is invalid (does not exist).
    InvalidFile(String),
    /// Failed to write the output
    WriteError(String),

    /// The program is malformed
    Malformed(String),

    /// The name has not been bound
    Undefined(Id, String),
    /// The name has already been bound.
    AlreadyBound(Id, String),

    /// A miscellaneous error. Should be replaced with a more precise error.
    #[allow(unused)]
    Misc(String),
}

/// Convience wrapper to represent success or meaningul compiler error.
pub type FilamentResult<T> = std::result::Result<T, Error>;

/// A span of the input program.
impl std::fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        use ErrorKind::*;
        match self {
            AlreadyBound(name, bound_by) => {
                write!(f, "Name `{}' is already bound by {}", name, bound_by)
            }
            Undefined(name, typ) => {
                write!(f, "Undefined {} name: {}", typ, name)
            }
            ParseError(err) => write!(f, "Filament Parser: {}", err),
            InvalidFile(msg) | Misc(msg) | WriteError(msg) => {
                write!(f, "{}", msg)
            }
            Malformed(msg) => write!(f, "{msg}"),
        }
    }
}

// Conversions from other error types to our error type so that
// we can use `?` in all the places.

impl From<std::str::Utf8Error> for Error {
    fn from(err: std::str::Utf8Error) -> Self {
        Error::invalid_file(err.to_string())
    }
}

impl From<pest_consume::Error<frontend::Rule>> for Error {
    fn from(e: pest_consume::Error<frontend::Rule>) -> Self {
        Error::parse_error(e)
    }
}

impl From<std::io::Error> for Error {
    fn from(_e: std::io::Error) -> Self {
        Error::write_error("IO Error".to_string())
    }
}

impl From<rsmt2::errors::Error> for Error {
    fn from(e: rsmt2::errors::Error) -> Self {
        Error::misc(format!("SMT Error: {}", e))
    }
}
