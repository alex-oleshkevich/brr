use brr_core::{Error, Secret};
use totp_rs::{Algorithm, Builder, Secret as Base32Secret, Totp};

const OTPAUTH_PREFIX: &str = "otpauth://";
const DIGITS: u8 = 6;
pub const STEP_SECONDS: u64 = 30;
const MAX_DIGITS: u8 = 9;

#[cfg(test)]
mod tests;

pub fn totp_check(secret: &str) -> Result<(), Error> {
    build(secret).map(drop)
}

pub fn totp_code(secret: &str, unix_s: u64) -> Result<Secret, Error> {
    Ok(Secret::new(build(secret)?.generate(unix_s).to_string()))
}

// The unchecked constructors are deliberate: many sites issue 80-bit secrets (16 base32
// characters), which the checked builder rejects as shorter than 128 bits.
fn build(secret: &str) -> Result<Totp, Error> {
    let totp = if secret.starts_with(OTPAUTH_PREFIX) {
        Totp::from_url_unchecked(secret).ok()
    } else {
        let base32: String = secret
            .chars()
            .filter(|c| *c != ' ')
            .collect::<String>()
            .to_uppercase();

        Base32Secret::try_from_base32(base32).ok().map(|key| {
            Builder::new()
                .with_algorithm(Algorithm::SHA1)
                .with_digits(DIGITS)
                .with_step_duration(STEP_SECONDS)
                .with_secret(key)
                .build_noncompliant()
        })
    };

    // generate() panics on 10+ digits and divides by the step; an empty key is no secret
    totp.filter(|totp| {
        totp.digits() <= MAX_DIGITS && totp.step() > 0 && !totp.secret().as_bytes().is_empty()
    })
    .ok_or_else(invalid_totp)
}

fn invalid_totp() -> Error {
    Error::InvalidArgument {
        field: "totp".into(),
        message: "expected an otpauth:// URI or a base32 secret".into(),
    }
}
