//! Journald drain for slog-rs
//!
//! Since Journald supports structured data, structured data passed to slog is
//! simply forwarded to Journald as structured data.
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

extern crate libc;
extern crate libsystemd_sys;
extern crate slog;

#[allow(deprecated, unused_imports)]
use std::ascii::AsciiExt;
use std::fmt::{Display, Formatter, Write};
use std::os::raw::{c_int, c_void};

use libc::{size_t, LOG_CRIT, LOG_DEBUG, LOG_ERR, LOG_INFO, LOG_NOTICE, LOG_WARNING};
use libsystemd_sys::const_iovec;
use libsystemd_sys::journal::sd_journal_sendv;
use slog::{Drain, Key, Level, OwnedKVList, Record, KV};

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
        serializer.add_field(format!("PRIORITY={}", level_to_priority(info.level())));
        serializer.add_field(format!("MESSAGE={}", info.msg()));
        serializer.add_field(format!("CODE_FILE={}", info.file()));
        serializer.add_field(format!("CODE_LINE={}", info.line()));
        serializer.add_field(format!("CODE_MODULE={}", info.module()));
        serializer.add_field(format!("CODE_FUNCTION={}", info.function()));

        logger_values.serialize(info, &mut serializer)?;
        info.kv().serialize(info, &mut serializer)?;

        journald_send(serializer.fields.as_slice())
    }
}

/// Error type for logging to journald.
#[derive(Debug)]
pub enum Error {
    /// Error representing a non-zero return from `sd_journal_sendv`.
    ///
    /// The contained integer is the return value form `sd_journal_sendv`, which can
    /// be treated as an errno.
    Journald(i32),
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

fn journald_send(args: &[String]) -> Result<(), Error> {
    let iovecs = strings_to_iovecs(args);
    let ret = unsafe { sd_journal_sendv(iovecs.as_ptr(), iovecs.len() as c_int) };
    if ret == 0 {
        Ok(())
    } else {
        // NOTE: journald returns a negative error code, so negate it to get the actual
        // error number
        Err(Error::Journald(-ret))
    }
}

fn level_to_priority(level: Level) -> c_int {
    match level {
        Level::Critical => LOG_CRIT,
        Level::Error => LOG_ERR,
        Level::Warning => LOG_WARNING,
        Level::Info => LOG_NOTICE,
        Level::Debug => LOG_INFO,
        Level::Trace => LOG_DEBUG,
    }
}

// NOTE: the resulting const_iovecs have the lifetime of
// the input strings
fn strings_to_iovecs(strings: &[String]) -> Vec<const_iovec> {
    strings
        .iter()
        .map(|s| const_iovec {
            iov_base: s.as_ptr() as *const c_void,
            iov_len: s.len() as size_t,
        })
        .collect()
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
    fields: Vec<String>,
}

impl Serializer {
    fn new() -> Serializer {
        Serializer { fields: Vec::new() }
    }
    /// Add field without sanitizing the key
    ///
    /// Note: if the key isn't a valid journald key name, it will be ignored.
    fn add_field(&mut self, field: String) {
        self.fields.push(field);
    }
    #[inline]
    fn emit<T: Display>(&mut self, key: Key, val: T) -> slog::Result {
        self.add_field(format!("{}={}", SanitizedKey(key), val));
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
