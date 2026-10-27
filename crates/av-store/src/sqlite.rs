//! SQLite's C API, as much of it as the store uses: a connection (`Db`)
//! and its prepared statements (`Stmt`), each closed on drop. The same on
//! both platforms; only the library linked differs.

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::path::Path;
use std::ptr::{self, NonNull};

use av_utils::error::{Error, Result};

#[repr(C)]
struct Sqlite3 {
    _opaque: [u8; 0],
}

#[repr(C)]
struct Sqlite3Stmt {
    _opaque: [u8; 0],
}

const SQLITE_OK: c_int = 0;
const SQLITE_CORRUPT: c_int = 11;
const SQLITE_TOOBIG: c_int = 18;
const SQLITE_MISUSE: c_int = 21;
const SQLITE_NOTADB: c_int = 26;
const SQLITE_ROW: c_int = 100;
const SQLITE_DONE: c_int = 101;

const SQLITE_OPEN_READWRITE: c_int = 0x02;
const SQLITE_OPEN_CREATE: c_int = 0x04;

/// A bind's destructor that has SQLite copy the value before the call
/// returns.
const SQLITE_TRANSIENT: isize = -1;

#[cfg_attr(target_os = "windows", link(name = "winsqlite3", kind = "raw-dylib"))]
#[cfg_attr(target_os = "linux", link(name = "sqlite3"))]
unsafe extern "system" {
    fn sqlite3_open_v2(
        filename: *const c_char,
        db: *mut *mut Sqlite3,
        flags: c_int,
        vfs: *const c_char,
    ) -> c_int;
    fn sqlite3_close(db: *mut Sqlite3) -> c_int;
    fn sqlite3_exec(
        db: *mut Sqlite3,
        sql: *const c_char,
        callback: *const c_void,
        arg: *mut c_void,
        errmsg: *mut *mut c_char,
    ) -> c_int;
    fn sqlite3_errmsg(db: *mut Sqlite3) -> *const c_char;
    fn sqlite3_prepare_v2(
        db: *mut Sqlite3,
        sql: *const c_char,
        bytes: c_int,
        stmt: *mut *mut Sqlite3Stmt,
        tail: *mut *const c_char,
    ) -> c_int;
    fn sqlite3_bind_int64(stmt: *mut Sqlite3Stmt, index: c_int, value: i64) -> c_int;
    fn sqlite3_bind_null(stmt: *mut Sqlite3Stmt, index: c_int) -> c_int;
    fn sqlite3_bind_text(
        stmt: *mut Sqlite3Stmt,
        index: c_int,
        text: *const c_char,
        bytes: c_int,
        destructor: isize,
    ) -> c_int;
    fn sqlite3_step(stmt: *mut Sqlite3Stmt) -> c_int;
    fn sqlite3_column_int64(stmt: *mut Sqlite3Stmt, column: c_int) -> i64;
    fn sqlite3_column_text(stmt: *mut Sqlite3Stmt, column: c_int) -> *const u8;
    fn sqlite3_column_bytes(stmt: *mut Sqlite3Stmt, column: c_int) -> c_int;
    fn sqlite3_reset(stmt: *mut Sqlite3Stmt) -> c_int;
    fn sqlite3_finalize(stmt: *mut Sqlite3Stmt) -> c_int;
}

// ----------------------------------------------------------------------------
/// Whether `err` says the file isn't a database, or a damaged one: the
/// only errors after which a catalog's file is made anew.
pub(crate) fn is_broken(err: &Error) -> bool {
    // The primary code is the low byte of an extended one.
    matches!(err, Error::Sqlite { code, .. } if matches!(code & 0xff, SQLITE_CORRUPT | SQLITE_NOTADB))
}

// ----------------------------------------------------------------------------
/// An open connection, closed on drop. Not `Send`: it stays on the thread
/// that opened it.
pub(crate) struct Db(NonNull<Sqlite3>);

impl Db {
    // ------------------------------------------------------------------------
    /// Open the database at `path`, made if it doesn't exist; `:memory:` is
    /// one in memory.
    pub(crate) fn open(path: &Path) -> Result<Db> {
        let invalid = || Error::InvalidPath {
            path: path.to_path_buf(),
        };
        let name = path.to_str().ok_or_else(invalid)?;
        let name = CString::new(name).map_err(|_| invalid())?;
        let mut raw = ptr::null_mut();
        let flags = SQLITE_OPEN_READWRITE | SQLITE_OPEN_CREATE;
        let code = unsafe { sqlite3_open_v2(name.as_ptr(), &mut raw, flags, ptr::null()) };
        // A failed open still hands back a connection, holding the message.
        let Some(db) = NonNull::new(raw).map(Db) else {
            return Err(Error::Sqlite {
                code,
                msg: "out of memory".to_string(),
            });
        };
        if code != SQLITE_OK {
            return Err(db.error(code));
        }
        Ok(db)
    }

    // ------------------------------------------------------------------------
    /// Run `sql`, one statement or several; rows they return are dropped.
    pub(crate) fn exec(&self, sql: &CStr) -> Result<()> {
        let code = unsafe {
            sqlite3_exec(
                self.0.as_ptr(),
                sql.as_ptr(),
                ptr::null(),
                ptr::null_mut(),
                ptr::null_mut(),
            )
        };
        self.check(code)
    }

    // ------------------------------------------------------------------------
    pub(crate) fn prepare(&self, sql: &CStr) -> Result<Stmt<'_>> {
        let mut raw = ptr::null_mut();
        let code = unsafe {
            sqlite3_prepare_v2(self.0.as_ptr(), sql.as_ptr(), -1, &mut raw, ptr::null_mut())
        };
        self.check(code)?;
        // SQL of nothing but comments and spaces prepares to no statement.
        let raw = NonNull::new(raw).ok_or_else(|| Error::Sqlite {
            code: SQLITE_MISUSE,
            msg: "no statement".to_string(),
        })?;
        Ok(Stmt { raw, db: self })
    }

    // ------------------------------------------------------------------------
    fn check(&self, code: c_int) -> Result<()> {
        if code == SQLITE_OK {
            Ok(())
        } else {
            Err(self.error(code))
        }
    }

    // ------------------------------------------------------------------------
    fn error(&self, code: c_int) -> Error {
        let msg = unsafe { CStr::from_ptr(sqlite3_errmsg(self.0.as_ptr())) };
        Error::Sqlite {
            code,
            msg: msg.to_string_lossy().into_owned(),
        }
    }
}

impl Drop for Db {
    fn drop(&mut self) {
        // Its statements borrow it, so they are finalized already.
        unsafe {
            sqlite3_close(self.0.as_ptr());
        }
    }
}

// ----------------------------------------------------------------------------
/// A prepared statement of `db`, finalized on drop. Parameters and columns
/// count as SQLite's do: parameters from 1, columns from 0.
pub(crate) struct Stmt<'db> {
    raw: NonNull<Sqlite3Stmt>,
    db: &'db Db,
}

impl Stmt<'_> {
    // ------------------------------------------------------------------------
    pub(crate) fn bind_i64(&mut self, index: c_int, value: i64) -> Result<()> {
        let code = unsafe { sqlite3_bind_int64(self.raw.as_ptr(), index, value) };
        self.db.check(code)
    }

    // ------------------------------------------------------------------------
    pub(crate) fn bind_null(&mut self, index: c_int) -> Result<()> {
        let code = unsafe { sqlite3_bind_null(self.raw.as_ptr(), index) };
        self.db.check(code)
    }

    // ------------------------------------------------------------------------
    pub(crate) fn bind_text(&mut self, index: c_int, text: &str) -> Result<()> {
        let bytes = c_int::try_from(text.len()).map_err(|_| Error::Sqlite {
            code: SQLITE_TOOBIG,
            msg: "string or blob too big".to_string(),
        })?;
        let code = unsafe {
            sqlite3_bind_text(
                self.raw.as_ptr(),
                index,
                text.as_ptr().cast(),
                bytes,
                SQLITE_TRANSIENT,
            )
        };
        self.db.check(code)
    }

    // ------------------------------------------------------------------------
    /// Run the statement to its next row: true with a row to read, false
    /// once it is done.
    pub(crate) fn step(&mut self) -> Result<bool> {
        match unsafe { sqlite3_step(self.raw.as_ptr()) } {
            SQLITE_ROW => Ok(true),
            SQLITE_DONE => Ok(false),
            code => Err(self.db.error(code)),
        }
    }

    // ------------------------------------------------------------------------
    pub(crate) fn column_i64(&self, column: c_int) -> i64 {
        unsafe { sqlite3_column_int64(self.raw.as_ptr(), column) }
    }

    // ------------------------------------------------------------------------
    /// The row's text in `column`; `None` for NULL, or for bytes that aren't
    /// UTF-8.
    pub(crate) fn column_text(&self, column: c_int) -> Option<String> {
        // The length is asked after the text, as SQLite's docs advise.
        let text = unsafe { sqlite3_column_text(self.raw.as_ptr(), column) };
        if text.is_null() {
            return None;
        }
        let bytes = unsafe { sqlite3_column_bytes(self.raw.as_ptr(), column) };
        let bytes = usize::try_from(bytes).ok()?;
        let text = unsafe { std::slice::from_raw_parts(text, bytes) };
        String::from_utf8(text.to_vec()).ok()
    }

    // ------------------------------------------------------------------------
    /// Ready the statement to run again; its parameters stay bound.
    pub(crate) fn reset(&mut self) -> Result<()> {
        let code = unsafe { sqlite3_reset(self.raw.as_ptr()) };
        self.db.check(code)
    }
}

impl Drop for Stmt<'_> {
    fn drop(&mut self) {
        unsafe {
            sqlite3_finalize(self.raw.as_ptr());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------------
    fn memory() -> Db {
        Db::open(Path::new(":memory:")).unwrap()
    }

    // ------------------------------------------------------------------------
    #[test]
    fn values_go_in_and_come_back() {
        let db = memory();
        db.exec(c"CREATE TABLE t(n INTEGER, s TEXT)").unwrap();
        let mut insert = db.prepare(c"INSERT INTO t VALUES(?1, ?2)").unwrap();
        for (n, s) in [(i64::MIN, "K\u{f6}ln"), (i64::MAX, "")] {
            insert.bind_i64(1, n).unwrap();
            insert.bind_text(2, s).unwrap();
            assert!(!insert.step().unwrap());
            insert.reset().unwrap();
        }
        insert.bind_i64(1, 0).unwrap();
        insert.bind_null(2).unwrap();
        assert!(!insert.step().unwrap());
        drop(insert);

        let mut select = db.prepare(c"SELECT n, s FROM t ORDER BY rowid").unwrap();
        let mut rows = Vec::new();
        while select.step().unwrap() {
            rows.push((select.column_i64(0), select.column_text(1)));
        }
        let expected = [
            (i64::MIN, Some("K\u{f6}ln".to_string())),
            (i64::MAX, Some(String::new())),
            (0, None),
        ];
        assert_eq!(rows, expected);
    }

    // ------------------------------------------------------------------------
    #[test]
    fn errors_carry_the_code_and_the_message() {
        let db = memory();
        let Err(Error::Sqlite { code, msg }) = db.exec(c"SELEC 1") else {
            panic!("bad SQL ran");
        };
        assert_eq!(code, 1);
        assert!(msg.contains("syntax error"), "{msg}");
        assert!(db.prepare(c"-- nothing").is_err());

        let broken = |code| Error::Sqlite {
            code,
            msg: String::new(),
        };
        assert!(is_broken(&broken(SQLITE_NOTADB)));
        assert!(is_broken(&broken(SQLITE_CORRUPT | (1 << 8))));
        assert!(!is_broken(&broken(5)));
    }
}
