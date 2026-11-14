use brr_core::{Error, PasswordCsvFormat, Secret, site_origin};

#[cfg(test)]
mod tests;

const BOM: char = '\u{feff}';
const URL: &str = "url";
const USERNAME: &str = "username";
const PASSWORD: &str = "password";
const NOTE: &str = "note";

#[derive(Debug)]
pub struct CsvLogin {
    pub site: String,
    pub login_url: String,
    pub username: String,
    pub password: Secret,
    pub note: Option<String>,
}

#[derive(Debug)]
pub struct CsvImport {
    pub logins: Vec<CsvLogin>,
    pub skipped: u32,
}

pub fn parse_password_csv(text: &str, format: PasswordCsvFormat) -> Result<CsvImport, Error> {
    let mut records = read_records(text.strip_prefix(BOM).unwrap_or(text))?.into_iter();
    let header = records.next().unwrap_or_default();
    let columns = Columns::find(&header, format)?;

    let mut logins = Vec::new();
    let mut skipped = 0;

    for record in records {
        match record_login(&record, header.len(), &columns) {
            Some(login) => logins.push(login),
            None => skipped += 1,
        }
    }

    Ok(CsvImport { logins, skipped })
}

struct Columns {
    url: usize,
    username: usize,
    password: usize,
    note: Option<usize>,
}

impl Columns {
    fn find(header: &[String], format: PasswordCsvFormat) -> Result<Columns, Error> {
        let (browser, required): (&str, &[&str]) = match format {
            PasswordCsvFormat::Chrome => ("Chrome", &["name", URL, USERNAME, PASSWORD]),
            PasswordCsvFormat::Firefox => ("Firefox", &[URL, USERNAME, PASSWORD]),
        };

        let mismatch = || Error::InvalidArgument {
            field: "file".into(),
            message: format!(
                "not a {browser} password export: expected columns {}",
                required.join(", ")
            ),
        };
        let index = |name: &str| {
            header
                .iter()
                .position(|column| column == name)
                .ok_or_else(mismatch)
        };

        for name in required {
            index(name)?;
        }

        Ok(Columns {
            url: index(URL)?,
            username: index(USERNAME)?,
            password: index(PASSWORD)?,
            note: index(NOTE)
                .ok()
                .filter(|_| format == PasswordCsvFormat::Chrome),
        })
    }
}

fn record_login(record: &[String], width: usize, columns: &Columns) -> Option<CsvLogin> {
    if record.len() != width {
        return None;
    }

    let login_url = &record[columns.url];
    let username = &record[columns.username];
    let password = &record[columns.password];

    if username.is_empty() || password.is_empty() {
        return None;
    }

    let note = columns
        .note
        .map(|index| record[index].clone())
        .filter(|note| !note.is_empty());

    Some(CsvLogin {
        site: site_origin(login_url)?,
        login_url: login_url.clone(),
        username: username.clone(),
        password: Secret::new(password.as_str()),
        note,
    })
}

// RFC 4180 subset: quoted fields may hold commas, doubled quotes and line breaks, so the
// text cannot be split into lines first. Blank lines yield no record. Fields past the header
// width are dropped: the row is already malformed, and this bounds memory on hostile input.
fn read_records(text: &str) -> Result<Vec<Vec<String>>, Error> {
    let mut records: Vec<Vec<String>> = Vec::new();
    let mut record = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            '"' if quoted => quoted = false,
            '"' if field.is_empty() => quoted = true,
            ',' if !quoted => {
                let field = std::mem::take(&mut field);
                if records
                    .first()
                    .is_none_or(|header| record.len() <= header.len())
                {
                    record.push(field);
                }
            }
            '\n' if !quoted => {
                record.push(std::mem::take(&mut field));
                end_record(&mut records, &mut record);
            }
            '\r' if !quoted => {}
            _ => field.push(c),
        }
    }

    if quoted {
        return Err(Error::InvalidArgument {
            field: "file".into(),
            message: "unterminated quoted field".into(),
        });
    }

    record.push(field);
    end_record(&mut records, &mut record);
    Ok(records)
}

fn end_record(records: &mut Vec<Vec<String>>, record: &mut Vec<String>) {
    let blank = record.len() == 1 && record[0].is_empty();
    let finished = std::mem::take(record);

    if !blank {
        records.push(finished);
    }
}
