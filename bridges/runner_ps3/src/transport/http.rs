//! The HTTP/1.0 client: one `GET` per connection, `Connection: close`,
//! and a pure response parser. A 404 is a value (the result file is
//! absent), not an error.
