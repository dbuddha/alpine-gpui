//! Tab-separated values with backslash escapes, so a field never holds a tab
//! or a newline and every file splits on tabs alone.

use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::Path;

/// The value written for a metric that a trial could not produce.
pub const MISSING: &str = "NA";

pub fn escape(field: &str) -> String {
    let mut escaped = String::with_capacity(field.len());
    for character in field.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            '\t' => escaped.push_str("\\t"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            other => escaped.push(other),
        }
    }
    escaped
}

pub fn unescape(field: &str) -> Result<String, String> {
    let mut plain = String::with_capacity(field.len());
    let mut characters = field.chars();
    while let Some(character) = characters.next() {
        if character != '\\' {
            plain.push(character);
            continue;
        }
        match characters.next() {
            Some('\\') => plain.push('\\'),
            Some('t') => plain.push('\t'),
            Some('n') => plain.push('\n'),
            Some('r') => plain.push('\r'),
            Some(other) => return Err(format!("unknown escape \\{other} in {field:?}")),
            None => return Err(format!("dangling backslash in {field:?}")),
        }
    }
    Ok(plain)
}

pub fn join<S: AsRef<str>>(fields: &[S]) -> String {
    fields
        .iter()
        .map(|field| escape(field.as_ref()))
        .collect::<Vec<_>>()
        .join("\t")
}

pub fn split(line: &str) -> Result<Vec<String>, String> {
    line.split('\t').map(unescape).collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Table {
    pub header: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

impl Table {
    pub fn new(header: &[&str]) -> Self {
        Self {
            header: header.iter().map(|name| (*name).to_owned()).collect(),
            rows: Vec::new(),
        }
    }

    pub fn push(&mut self, row: Vec<String>) -> Result<(), String> {
        if row.len() != self.header.len() {
            return Err(format!(
                "row has {} fields, header has {}",
                row.len(),
                self.header.len()
            ));
        }
        self.rows.push(row);
        Ok(())
    }

    pub fn render(&self) -> String {
        let mut text = join(&self.header);
        text.push('\n');
        for row in &self.rows {
            text.push_str(&join(row));
            text.push('\n');
        }
        text
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        let mut lines = text.lines();
        let header = split(lines.next().ok_or("empty table")?)?;
        let mut table = Self {
            header,
            rows: Vec::new(),
        };
        for (index, line) in lines.enumerate() {
            if line.is_empty() {
                continue;
            }
            table
                .push(split(line)?)
                .map_err(|error| format!("line {}: {error}", index + 2))?;
        }
        Ok(table)
    }

    pub fn column(&self, name: &str) -> Result<usize, String> {
        self.header
            .iter()
            .position(|field| field == name)
            .ok_or_else(|| format!("missing column {name}"))
    }

    pub fn get<'row>(&self, row: &'row [String], name: &str) -> Result<&'row str, String> {
        let index = self.column(name)?;
        row.get(index)
            .map(String::as_str)
            .ok_or_else(|| format!("row is missing column {name}"))
    }
}

pub fn write(path: &Path, table: &Table) -> Result<(), String> {
    fs::write(path, table.render()).map_err(|error| format!("write {}: {error}", path.display()))
}

pub fn read(path: &Path) -> Result<Table, String> {
    let text =
        fs::read_to_string(path).map_err(|error| format!("read {}: {error}", path.display()))?;
    Table::parse(&text).map_err(|error| format!("{}: {error}", path.display()))
}

/// Appends rows, writing the header first for a new or empty file and
/// refusing a file whose header differs.
pub fn append(path: &Path, header: &[&str], rows: &[Vec<String>]) -> Result<(), String> {
    let existing = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(format!("read {}: {error}", path.display())),
    };
    let mut text = String::new();
    if existing.is_empty() {
        text.push_str(&join(header));
        text.push('\n');
    } else {
        let first = existing.lines().next().unwrap_or_default();
        if first != join(header) {
            return Err(format!("{} has a different header", path.display()));
        }
        if !existing.ends_with('\n') {
            text.push('\n');
        }
    }
    for row in rows {
        if row.len() != header.len() {
            return Err(format!(
                "row has {} fields, header has {}",
                row.len(),
                header.len()
            ));
        }
        text.push_str(&join(row));
        text.push('\n');
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| format!("open {}: {error}", path.display()))?;
    file.write_all(text.as_bytes())
        .map_err(|error| format!("append {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::{Table, append, escape, join, split, unescape};

    #[test]
    fn escaping_round_trips_control_characters() -> Result<(), String> {
        for field in ["plain", "a\tb", "line\nnext", "back\\slash", "\r\n\t\\", ""] {
            assert_eq!(unescape(&escape(field))?, field);
            assert!(!escape(field).contains('\t'));
            assert!(!escape(field).contains('\n'));
        }
        Ok(())
    }

    #[test]
    fn malformed_escapes_are_rejected() {
        assert!(unescape("bad\\x").is_err());
        assert!(unescape("dangling\\").is_err());
    }

    #[test]
    fn lines_split_on_tabs_only() -> Result<(), String> {
        let line = join(&["a b", "c\td", "e"]);
        assert_eq!(split(&line)?, vec!["a b", "c\td", "e"]);
        Ok(())
    }

    #[test]
    fn tables_round_trip_and_check_width() -> Result<(), String> {
        let mut table = Table::new(&["name", "value"]);
        table.push(vec!["x".to_owned(), "1".to_owned()])?;
        assert!(table.push(vec!["short".to_owned()]).is_err());
        let parsed = Table::parse(&table.render())?;
        assert_eq!(parsed, table);
        assert_eq!(parsed.get(&parsed.rows[0], "value")?, "1");
        assert!(parsed.column("absent").is_err());
        assert!(Table::parse("a\tb\nonly-one\n").is_err());
        Ok(())
    }

    #[test]
    fn append_writes_header_once_and_refuses_a_foreign_header() -> Result<(), String> {
        let dir = crate::test_dir("tsv-append")?;
        let path = dir.join("rows.tsv");
        let _ = std::fs::remove_file(&path);
        append(&path, &["a", "b"], &[vec!["1".to_owned(), "2".to_owned()]])?;
        append(&path, &["a", "b"], &[vec!["3".to_owned(), "4".to_owned()]])?;
        let text = std::fs::read_to_string(&path).map_err(|error| error.to_string())?;
        assert_eq!(text, "a\tb\n1\t2\n3\t4\n");
        assert!(append(&path, &["a", "c"], &[]).is_err());
        assert!(append(&path, &["a", "b"], &[vec!["only".to_owned()]]).is_err());
        std::fs::remove_dir_all(&dir).map_err(|error| error.to_string())?;
        Ok(())
    }
}
