//! SQL console: run a query against PostgreSQL or MySQL/MariaDB on the
//! server or inside one of its containers, and get the rows back as a table.
//!
//! The query goes over stdin, never into the command line; the script is
//! fixed except for the database name (checked, then quoted). It connects the
//! way the Databases tab does: as `postgres` / MySQL root over the socket, or
//! with the container image's own login. Read-only mode makes the session's
//! transactions read-only before the query runs, so writes are refused by the
//! database itself. The activity log records the statement kind, size and a
//! hash, not the text, which may contain passwords or personal data.

use crate::databases::{validate_db_name, DbKind};
use crate::error::{Error, Result};
use crate::shell::quote;
use serde::Serialize;
use sha2::{Digest, Sha256};

/// Bytes of output kept; more is cut off (and said so).
pub const MAX_OUTPUT: usize = 4_000_000;
pub const MAX_ROWS: usize = 5_000;
pub const MAX_QUERY: usize = 200_000;

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SqlResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Option<String>>>,
    /// Output that isn't one table (several statements' results, messages).
    pub text: Option<String>,
    /// What the database said went wrong.
    pub error: Option<String>,
    pub truncated: bool,
    pub duration_ms: i64,
}

pub fn validate_query(q: &str) -> Result<()> {
    if q.trim().is_empty() {
        return Err(Error::InvalidInput("write a query first".into()));
    }
    if q.len() > MAX_QUERY || q.contains('\0') {
        return Err(Error::InvalidInput("the query is too long".into()));
    }
    Ok(())
}

/// The first keyword (`SELECT`, `UPDATE`, …), for the activity log.
pub fn statement(q: &str) -> String {
    let mut rest = q.trim_start();
    loop {
        if let Some(r) = rest.strip_prefix("--") {
            rest = r.split_once('\n').map(|x| x.1).unwrap_or("").trim_start();
        } else if let Some(r) = rest.strip_prefix("/*") {
            rest = r.split_once("*/").map(|x| x.1).unwrap_or("").trim_start();
        } else {
            break;
        }
    }
    rest.split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .next()
        .unwrap_or("")
        .to_ascii_uppercase()
}

pub fn fingerprint(q: &str) -> String {
    hex::encode(&Sha256::digest(q.as_bytes())[..8])
}

/// The script: runs the client with the query from stdin into a temp file,
/// then prints the exit code, stderr and (at most MAX_OUTPUT bytes of) stdout.
pub fn script(kind: DbKind, db: &str, read_only: bool) -> Result<String> {
    let db = quote(validate_db_name(db)?);
    let run = match kind {
        DbKind::Postgres => format!(
            "command -v psql >/dev/null 2>&1 || {{ echo @@missing; exit 0; }}\n\
             as_pg() {{ if [ -n \"$ORBITAL_CONTAINER\" ]; then \"$@\"; elif [ \"$(id -u)\" = 0 ] && command -v runuser >/dev/null 2>&1; then runuser -u postgres -- \"$@\"; elif sudo -n -u postgres true 2>/dev/null; then sudo -n -u postgres -- \"$@\"; else \"$@\"; fi; }}\n\
             as_pg psql -X -q -v ON_ERROR_STOP=1 --csv -P footer=off -d {db} {ro} -f - >\"$o\" 2>\"$e\"",
            ro = if read_only { "-c 'SET default_transaction_read_only = on'" } else { "" }
        ),
        DbKind::Mysql => format!(
            "M=$(command -v mysql || command -v mariadb) || {{ echo @@missing; exit 0; }}\n\
             {{ {ro} cat; }} | \"$M\" --batch --database={db} >\"$o\" 2>\"$e\"",
            ro = if read_only { "echo 'SET SESSION TRANSACTION READ ONLY;';" } else { "" }
        ),
        DbKind::Mongo => return Err(Error::InvalidInput("the SQL console works with PostgreSQL and MySQL".into())),
    };
    Ok(format!(
        "export LC_ALL=C.UTF-8 2>/dev/null || export LC_ALL=C\n\
         o=$(mktemp) && e=$(mktemp) || exit 1\n\
         trap 'rm -f \"$o\" \"$e\"' EXIT\n\
         {run}\n\
         rc=$?\n\
         echo \"@@exit $rc\"; echo \"@@size $(wc -c < \"$o\")\"\n\
         echo @@err; head -c 20000 \"$e\"; echo\n\
         echo @@out; head -c {MAX_OUTPUT} \"$o\"\n"
    ))
}

/// RFC 4180 CSV (psql --csv): quoted fields may hold commas, quotes and newlines.
pub fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    let mut any = false;
    while let Some(c) = chars.next() {
        any = true;
        if quoted {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    field.push('"');
                    chars.next();
                }
                '"' => quoted = false,
                c => field.push(c),
            }
        } else {
            match c {
                '"' => quoted = true,
                ',' => row.push(std::mem::take(&mut field)),
                '\r' => {}
                '\n' => {
                    row.push(std::mem::take(&mut field));
                    rows.push(std::mem::take(&mut row));
                    any = false;
                }
                c => field.push(c),
            }
        }
    }
    if any {
        row.push(field);
        rows.push(row);
    }
    rows
}

/// MySQL --batch: tab-separated, `\t` `\n` `\\` `\0` escaped, NULL as `NULL`.
pub fn parse_tsv(text: &str) -> Vec<Vec<Option<String>>> {
    text.lines()
        .map(|l| {
            l.split('\t')
                .map(|f| {
                    if f == "NULL" {
                        return None;
                    }
                    let mut out = String::new();
                    let mut it = f.chars();
                    while let Some(c) = it.next() {
                        if c == '\\' {
                            match it.next() {
                                Some('t') => out.push('\t'),
                                Some('n') => out.push('\n'),
                                Some('0') => out.push('\0'),
                                Some('\\') => out.push('\\'),
                                Some(o) => {
                                    out.push('\\');
                                    out.push(o)
                                }
                                None => out.push('\\'),
                            }
                        } else {
                            out.push(c);
                        }
                    }
                    Some(out)
                })
                .collect()
        })
        .collect()
}

pub fn parse(kind: DbKind, out: &str, duration_ms: i64) -> Result<SqlResult> {
    if out.lines().next().is_some_and(|l| l.trim() == "@@missing") {
        return Err(Error::NotFound(format!(
            "{}'s command-line client isn't installed there",
            kind.label()
        )));
    }
    let code: i32 = out
        .lines()
        .find_map(|l| l.strip_prefix("@@exit "))
        .and_then(|v| v.trim().parse().ok())
        .ok_or_else(|| Error::Other("the database didn't answer".into()))?;
    let size: usize = out
        .lines()
        .find_map(|l| l.strip_prefix("@@size "))
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0);
    let err = out
        .split_once("\n@@err\n")
        .map(|x| x.1)
        .and_then(|r| r.split_once("\n@@out\n").map(|x| x.0))
        .unwrap_or("")
        .trim()
        .to_string();
    let body = out.split_once("\n@@out\n").map(|x| x.1).unwrap_or("");
    let mut r = SqlResult {
        duration_ms,
        truncated: size > MAX_OUTPUT,
        ..Default::default()
    };
    if code != 0 {
        r.error = Some(if err.is_empty() {
            format!("the client stopped with code {code}")
        } else {
            err
        });
        if !body.trim().is_empty() {
            r.text = Some(body.to_string());
        }
        return Ok(r);
    }
    let table: Vec<Vec<Option<String>>> = match kind {
        DbKind::Postgres => parse_csv(body).into_iter().map(|row| row.into_iter().map(Some).collect()).collect(),
        _ => parse_tsv(body),
    };
    let width = table.first().map(Vec::len).unwrap_or(0);
    if width > 0 && table.iter().all(|row| row.len() == width) {
        let mut it = table.into_iter();
        r.columns = it.next().unwrap().into_iter().map(|c| c.unwrap_or_default()).collect();
        let rows: Vec<_> = it.collect();
        if rows.len() > MAX_ROWS {
            r.truncated = true;
        }
        r.rows = rows.into_iter().take(MAX_ROWS).collect();
    } else if !body.trim().is_empty() {
        r.text = Some(body.to_string());
    }
    if !err.is_empty() {
        // Notices and warnings that came with a successful run.
        let t = r.text.take().unwrap_or_default();
        r.text = Some(format!("{err}{}{t}", if t.is_empty() { "" } else { "\n" }));
    }
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statement_kind_skips_comments() {
        assert_eq!(statement("  -- why\n/* c */ select 1"), "SELECT");
        assert_eq!(statement("UPDATE t SET a=1"), "UPDATE");
        assert_eq!(statement("with x as (select 1) delete from t"), "WITH");
        assert_eq!(fingerprint("select 1").len(), 16);
    }

    #[test]
    fn csv_with_quotes_commas_and_newlines() {
        let rows = parse_csv("id,name,note\n1,\"Smith, Anna\",\"said \"\"hi\"\"\nand left\"\n2,Bob,\n");
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[1], ["1", "Smith, Anna", "said \"hi\"\nand left"]);
        assert_eq!(rows[2], ["2", "Bob", ""]);
    }

    #[test]
    fn tsv_with_escapes_and_nulls() {
        let rows = parse_tsv("id\tnote\n1\ta\\tb\\nc\n2\tNULL\n");
        assert_eq!(rows[1], [Some("1".into()), Some("a\tb\nc".into())]);
        assert_eq!(rows[2][1], None);
    }

    fn out(code: i32, err: &str, body: &str) -> String {
        format!("@@exit {code}\n@@size {}\n@@err\n{err}\n@@out\n{body}", body.len())
    }

    #[test]
    fn parses_tables_errors_and_text() {
        let r = parse(DbKind::Postgres, &out(0, "", "id,name\n1,anna\n2,bob\n"), 12).unwrap();
        assert_eq!(r.columns, ["id", "name"]);
        assert_eq!(r.rows.len(), 2);
        let e = parse(
            DbKind::Postgres,
            &out(3, "ERROR:  cannot execute UPDATE in a read-only transaction", ""),
            5,
        )
        .unwrap();
        assert!(e.error.unwrap().contains("read-only"));
        let m = parse(DbKind::Mysql, &out(0, "", "a\tb\n1\t2\nc\n3\n"), 1).unwrap();
        assert!(m.columns.is_empty() && m.text.is_some(), "two result sets aren't one table");
        let n = parse(DbKind::Postgres, &out(0, "NOTICE:  table does not exist, skipping", ""), 1).unwrap();
        assert!(n.text.unwrap().contains("NOTICE"));
        assert!(matches!(parse(DbKind::Mysql, "@@missing\n", 0), Err(Error::NotFound(_))));
        let big = format!("@@exit 0\n@@size {}\n@@err\n\n@@out\nx\n1\n", MAX_OUTPUT + 5);
        assert!(parse(DbKind::Postgres, &big, 0).unwrap().truncated);
    }

    #[test]
    fn scripts_quote_the_database_and_never_carry_the_query() {
        let s = script(DbKind::Postgres, "shop", true).unwrap();
        assert!(s.contains("-d shop -c 'SET default_transaction_read_only = on' -f -"));
        assert!(script(DbKind::Postgres, "x; rm -rf /", false).is_err());
        assert!(script(DbKind::Mysql, "shop", true).unwrap().contains("READ ONLY"));
        assert!(!script(DbKind::Mysql, "shop", false).unwrap().contains("READ ONLY"));
        assert!(script(DbKind::Mongo, "x", false).is_err());
        assert!(validate_query("  ").is_err());
    }

    // Runs the script meant for Linux servers through a local sh.
    #[cfg(unix)]
    #[test]
    fn scripts_are_valid_sh() {
        for k in [DbKind::Postgres, DbKind::Mysql] {
            for ro in [true, false] {
                let s = script(k, "shop", ro).unwrap();
                let o = std::process::Command::new("sh").arg("-n").arg("-c").arg(&s).output().unwrap();
                assert!(o.status.success(), "{s}\n{}", String::from_utf8_lossy(&o.stderr));
            }
        }
    }
}
