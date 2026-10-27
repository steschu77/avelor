//! What Avelor keeps beside a vault: its catalog (`catalog.rs`), a row per
//! file of the index, for what the files and memory can't do quickly --
//! reading only what changed, searching -- and nothing only it holds:
//! deleting it costs a full read. In SQLite, the system's own
//! (`winsqlite3.dll` on Windows, libsqlite3 on Linux), through a binding of
//! the few functions it uses (`sqlite.rs`). Its types are plain; it knows
//! no vault.

pub mod catalog;
mod sqlite;
