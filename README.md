# slog-journald

[![Build Status](https://travis-ci.org/slog-rs/journald.svg?branch=master)](https://travis-ci.org/slog-rs/journald)
[![Gitter](https://img.shields.io/gitter/room/slog-rs/slog.svg)](https://gitter.im/slog-rs/slog)
[![Documentation](https://docs.rs/slog/badge.svg)](https://docs.rs/releases/search?query=slog)

This is a straightforward journald drain for [slog-rs](https://github.com/dpc/slog-rs).

Journald and slog-rs work very well together since both support structured log data. This crate will convert structured data (that is, key-value pairs) into journald fields. Since, journald field names are more restrictive than keys in slog-rs, key names are sanitized to be valid journald fields.
