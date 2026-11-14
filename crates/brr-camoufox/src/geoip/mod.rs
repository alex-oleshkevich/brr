#[cfg(test)]
mod tests;

use std::{net::IpAddr, path::Path};

use brr_core::Error;
use maxminddb::{LookupResult, MaxMindDbError, Mmap, PathElement, Reader};
use serde::Deserialize;

use crate::locales;

// The field paths of the GeoIP AIO database by daijro
const COUNTRY_PATH: [&str; 2] = ["country", "iso_code"];
const LONGITUDE_PATH: [&str; 2] = ["location", "longitude"];
const LATITUDE_PATH: [&str; 2] = ["location", "latitude"];
const TIMEZONE_PATH: [&str; 2] = ["location", "time_zone"];

#[derive(Debug, Clone, PartialEq)]
pub struct Geo {
    pub ip: IpAddr,
    pub country: String,
    pub timezone: String,
    pub latitude: f64,
    pub longitude: f64,
    pub locale: String,
}

pub struct GeoDb {
    reader: Reader<Mmap>,
}

fn invalid(message: String) -> Error {
    Error::InvalidArgument {
        field: "geoip".to_owned(),
        message,
    }
}

fn corrupt(error: &MaxMindDbError) -> Error {
    invalid(format!(
        "the GeoIP database is damaged ({error}); install it again"
    ))
}

fn field<'de, T: Deserialize<'de>>(
    found: &LookupResult<'de, Mmap>,
    keys: &[&str; 2],
) -> Result<T, Error> {
    let elements = keys.map(PathElement::Key);
    found
        .decode_path(&elements)
        .map_err(|error| corrupt(&error))?
        .ok_or_else(|| {
            invalid(format!(
                "the GeoIP record has no {}; install the GeoIP AIO database",
                keys.join(".")
            ))
        })
}

impl GeoDb {
    pub fn open(path: &Path) -> Result<GeoDb, Error> {
        // SAFETY: the file is only ever replaced by renaming a finished one over it
        // (brr_builds::install_geoip), which leaves this mapping on the old inode, so it is
        // never written or truncated while mapped.
        let reader = unsafe { Reader::open_mmap(path) }.map_err(|error| match error {
            MaxMindDbError::Io(error) => invalid(format!(
                "cannot read the GeoIP database {}: {error}; install it first",
                path.display()
            )),
            error => corrupt(&error),
        })?;

        Ok(GeoDb { reader })
    }

    pub fn lookup(&self, ip: IpAddr) -> Result<Geo, Error> {
        let found = self.reader.lookup(ip).map_err(|error| corrupt(&error))?;
        if !found.has_data() {
            return Err(invalid(format!("{ip} is not in the GeoIP database")));
        }

        let country: String = field(&found, &COUNTRY_PATH)?;
        let longitude = field(&found, &LONGITUDE_PATH)?;
        let latitude = field(&found, &LATITUDE_PATH)?;
        let timezone = field(&found, &TIMEZONE_PATH)?;

        let country = country.to_uppercase();
        let locale = locales::from_region(&country, None)
            .map_err(|_| invalid(format!("no language is known for the country {country}")))?;
        Ok(Geo {
            ip,
            locale: locale.as_string(),
            country,
            timezone,
            latitude,
            longitude,
        })
    }
}
