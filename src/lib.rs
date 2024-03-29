//! Journald drain for slog-rs
//!
//! Since Journald supports structured data, structured data passed to slog is
//! simply forwarded to Journald as structured data.
//!
//! This crate supports specialized handling of logged errors via features.
//! Look into `Cargo.toml` for more information.
//!
//! # Examples
//! ```
//! #[macro_use]
//! extern crate slog;
//! extern crate slog_journald;
//!
//! use slog::*;
//! use slog_journald::*;
//!
//! fn main() {
//!     let root = Logger::root(JournaldDrain.ignore_res(), o!("build_di" => "12344"));
//!     info!(root, "Testing journald"; "foo" => "bar");
//! }
//! ```

#![warn(missing_docs)]

extern crate libsystemd;
extern crate slog;

#[allow(deprecated, unused_imports)]
use std::ascii::AsciiExt;
use std::fmt;
use std::fmt::{Display, Formatter, Write};

use libsystemd::errors::SdError;
use libsystemd::logging::{journal_send, Priority};
use slog::{Drain, Key, Level, OwnedKVList, Record, KV};
use std::borrow::Cow;

/// Drain records and send to journald as structured data.
///
/// Journald requires keys to be uppercase alphanumeric, so logging keys
/// are capitalized and all non-alpha-numeric letters are converted to underscores.
pub struct JournaldDrain;

impl Drain for JournaldDrain {
    type Ok = ();
    type Err = ::Error;

    fn log(&self, info: &Record, logger_values: &OwnedKVList) -> Result<(), ::Error> {
        let mut serializer = Serializer::new();
        serializer.add_field(Cow::Borrowed("CODE_FILE"), info.file().to_string());
        serializer.add_field(Cow::Borrowed("CODE_LINE"), info.line().to_string());
        serializer.add_field(Cow::Borrowed("CODE_MODULE"), info.module().to_string());
        serializer.add_field(Cow::Borrowed("CODE_FUNCTION"), info.function().to_string());

        logger_values.serialize(info, &mut serializer)?;
        info.kv().serialize(info, &mut serializer)?;

        journal_send(
            level_to_priority(info.level()),
            &format!("{}", info.msg()),
            serializer.fields.into_iter(),
        )
        .map_err(Error::Journald)
    }
}

/// Error type for logging to journald.
#[derive(Debug)]
pub enum Error {
    /// Error representing a non-zero return from `sd_journal_sendv`.
    ///
    /// The contained integer is the return value form `sd_journal_sendv`, which can
    /// be treated as an errno.
    Journald(SdError),
    /// Error from serializing
    Serialization(slog::Error),
}

impl Display for Error {
    fn fmt(&self, fmt: &mut Formatter) -> std::fmt::Result {
        match *self {
            Error::Journald(ref errno) => write!(fmt, "sd_journal_sendv returned {}", errno),
            Error::Serialization(ref e) => write!(fmt, "Unable to serialize item: {:?}", e),
        }
    }
}

impl std::error::Error for Error {
    #[allow(deprecated)] // using std::error::Error::description : deprecated since rust 1.42.0
    fn description(&self) -> &str {
        match *self {
            Error::Journald(_) => "Unable to send to journald",
            Error::Serialization(ref e) => e.description(),
        }
    }

    fn cause(&self) -> Option<&dyn std::error::Error> {
        match *self {
            Error::Journald(_) => None,
            Error::Serialization(ref e) => Some(e),
        }
    }
}

impl From<slog::Error> for Error {
    fn from(e: slog::Error) -> Error {
        Error::Serialization(e)
    }
}

fn level_to_priority(level: Level) -> Priority {
    match level {
        Level::Critical => Priority::Critical,
        Level::Error => Priority::Error,
        Level::Warning => Priority::Warning,
        Level::Info => Priority::Notice,
        Level::Debug => Priority::Info,
        Level::Trace => Priority::Debug,
    }
}

/// Journald keys must consist only of uppercase letters, numbers
/// and underscores (but cannot begin with underscores).
/// So we capitalize the string and replace any invalid characters with underscores
struct SanitizedKey(Key);

impl<'a> Display for SanitizedKey {
    fn fmt(&self, fmt: &mut Formatter) -> std::fmt::Result {
        // Until we find a non-underscore character, we can't output underscores for any other chars
        let mut found_non_underscore = false;
        #[cfg_attr(not(feature = "slog/dynamic-keys"), allow(clippy::useless_asref))]
        let key: &str = self.0.as_ref();
        for c in key.chars() {
            match c {
                'A'..='Z' | '0'..='9' => {
                    fmt.write_char(c)?;
                    found_non_underscore = true;
                }
                'a'..='z' => {
                    fmt.write_char(c.to_ascii_uppercase())?;
                    found_non_underscore = true;
                }
                _ if found_non_underscore => fmt.write_char('_')?,
                _ => {}
            }
        }
        Ok(())
    }
}

struct Serializer {
    fields: Vec<(Cow<'static, str>, String)>,
}

impl Serializer {
    fn new() -> Serializer {
        Serializer { fields: Vec::new() }
    }
    /// Add field without sanitizing the key
    ///
    /// Note: if the key isn't a valid journald key name, it will be ignored.
    fn add_field(&mut self, key: Cow<'static, str>, value: String) {
        self.fields.push((key, value));
    }

    #[inline]
    #[allow(clippy::unnecessary_wraps)]
    fn emit<T: Display>(&mut self, key: Key, val: T) -> slog::Result {
        self.add_field(Cow::Owned(SanitizedKey(key).to_string()), val.to_string());
        Ok(())
    }
}

macro_rules! __emitter {
    ($name:ident : $T:ty) => {
        fn $name(&mut self, key: Key, val: $T) -> slog::Result {
            self.emit(key, val)
        }
    };
    ($name:ident = $val:expr) => {
        fn $name(&mut self, key: Key) -> slog::Result {
            self.emit(key, $val)
        }
    };
}

impl slog::Serializer for Serializer {
    __emitter!(emit_unit = "");
    __emitter!(emit_none = "None");

    __emitter!(emit_bool: bool);
    __emitter!(emit_char: char);
    __emitter!(emit_u8: u8);
    __emitter!(emit_i8: i8);
    __emitter!(emit_u16: u16);
    __emitter!(emit_i16: i16);
    __emitter!(emit_u32: u32);
    __emitter!(emit_i32: i32);
    __emitter!(emit_u64: u64);
    __emitter!(emit_i64: i64);
    __emitter!(emit_f32: f32);
    __emitter!(emit_f64: f64);
    __emitter!(emit_usize: usize);
    __emitter!(emit_isize: isize);
    __emitter!(emit_str: &str);
    __emitter!(emit_arguments: &std::fmt::Arguments);

    fn emit_error(&mut self, key: Key, error: &(dyn std::error::Error + 'static)) -> slog::Result {
        #[cfg(feature = "log_errno")]
        {
            let mut error_source = Some(error);
            while let Some(source) = error_source {
                if let Some(io_error) = source.downcast_ref::<std::io::Error>() {
                    if let Some(errno) = io_error.raw_os_error() {
                        self.add_field(Cow::Borrowed("ERRNO"), errno.to_string());
                    }
                }
                error_source = source.source();
            }
        }
        #[cfg(feature = "log_error_sources")]
        {
            let mut error_cause = Some(error);
            let mut depth = 0usize;
            while let Some(cause) = error_cause {
                self.add_field(
                    Cow::Owned(format!("ERROR_SOURCE_{}", depth)),
                    cause.to_string(),
                );
                depth += 1;
                error_cause = cause.cause();
            }
            self.add_field(Cow::Borrowed("ERROR_SOURCE_DEPTH"), depth.to_string());
        }

        self.emit_arguments(key, &format_args!("{}", ErrorAsFmt(error)))
    }
}

// copied from slog
struct ErrorAsFmt<'a>(pub &'a (dyn std::error::Error + 'static));

impl<'a> fmt::Display for ErrorAsFmt<'a> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // For backwards compatibility
        // This is fine because we don't need downcasting
        #![allow(deprecated)]
        write!(f, "{}", self.0)?;
        let mut error = self.0.cause();
        while let Some(source) = error {
            write!(f, ": {}", source)?;
            error = source.cause();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizer_no_leading_underscores() {
        assert_eq!(SanitizedKey("_A".into()).to_string(), "A");
        assert_eq!(SanitizedKey("__A".into()).to_string(), "A");
    }

    #[test]
    fn sanitizer_allow_inner_underscore() {
        assert_eq!(SanitizedKey("A_A".into()).to_string(), "A_A");
        assert_eq!(SanitizedKey("A__A".into()).to_string(), "A__A");
        assert_eq!(SanitizedKey("A__A_".into()).to_string(), "A__A_");
    }

    #[test]
    fn sanitizer_uppercases() {
        assert_eq!(SanitizedKey("abcde".into()).to_string(), "ABCDE");
        assert_eq!(SanitizedKey("aBcDe".into()).to_string(), "ABCDE");
        assert_eq!(SanitizedKey("a123b".into()).to_string(), "A123B");
        assert_eq!(SanitizedKey("A123B".into()).to_string(), "A123B");
    }

    #[test]
    fn sanitizer_replaces_chars_with_underscores() {
        assert_eq!(
            SanitizedKey("A `~!@#$%^&*()-_=+A".into()).to_string(),
            "A_________________A"
        );
        assert_eq!(SanitizedKey("A\u{ABCD}A".into()).to_string(), "A_A");
        assert_eq!(SanitizedKey("A\t".into()).to_string(), "A_");
    }

    #[test]
    fn sanitizer_cant_replace_starting_symbols_with_underscores() {
        assert_eq!(SanitizedKey("!A".into()).to_string(), "A");
        assert_eq!(SanitizedKey("!*".into()).to_string(), "");
        assert_eq!(SanitizedKey("(A)".into()).to_string(), "A_");
    }
}
