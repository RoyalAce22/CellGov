//! One runner per console at a time: a lease file in the temp
//! directory that names the holder, refused with the command that
//! releases it, and released on drop.
