use aes::Aes256;
use base64::{Engine, engine::general_purpose::STANDARD};
use cbc::cipher::block_padding::Pkcs7;
use cbc::cipher::{BlockModeEncrypt, KeyIvInit};
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use test_case::test_case;

use super::{Error, Kdf, Key, decrypt, master_key, stretch};

// Vectors from Bitwarden's SDK (bitwarden-crypto keys/kdf.rs and keys/utils.rs tests).
const SDK_PASSWORD: &str = "67t9b5g67$%Dh89n";
const SDK_SALT: &str = "test_key";
const SDK_PBKDF2_KEY: [u8; 32] = [
    31, 79, 104, 226, 150, 71, 177, 90, 194, 80, 172, 209, 17, 129, 132, 81, 138, 167, 69, 167,
    254, 149, 2, 27, 39, 197, 64, 42, 22, 195, 86, 75,
];
const SDK_ARGON2_KEY: [u8; 32] = [
    207, 240, 225, 177, 162, 19, 163, 76, 98, 106, 179, 175, 224, 9, 17, 240, 20, 147, 237, 47,
    246, 150, 141, 184, 62, 225, 131, 242, 51, 53, 225, 242,
];
const SDK_STRETCHED_ENC: [u8; 32] = [
    111, 31, 178, 45, 238, 152, 37, 114, 143, 215, 124, 83, 135, 173, 195, 23, 142, 134, 120, 249,
    61, 132, 163, 182, 113, 197, 189, 204, 188, 21, 237, 96,
];
const SDK_STRETCHED_MAC: [u8; 32] = [
    221, 127, 206, 234, 101, 27, 202, 38, 86, 52, 34, 28, 78, 28, 185, 16, 48, 61, 127, 166, 209,
    247, 194, 87, 232, 26, 48, 85, 193, 249, 179, 155,
];

const KEY_BYTES: [u8; 64] = [9; 64];
const OTHER_KEY_BYTES: [u8; 64] = [8; 64];
const IV: [u8; 16] = [7; 16];
const PLAIN: &str = "correct horse battery staple";

pub(crate) fn encrypt(plain: &[u8], key: &Key) -> String {
    let data = cbc::Encryptor::<Aes256>::new_from_slices(key.enc(), &IV)
        .unwrap()
        .encrypt_padded_vec::<Pkcs7>(plain);
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(key.mac()).unwrap();
    mac.update(&IV);
    mac.update(&data);
    format!(
        "2.{}|{}|{}",
        STANDARD.encode(IV),
        STANDARD.encode(&data),
        STANDARD.encode(mac.finalize().into_bytes())
    )
}

#[test_case(&Kdf::Pbkdf2 { iterations: 10_000 }, SDK_PBKDF2_KEY ; "pbkdf2")]
#[test_case(&Kdf::Argon2id { iterations: 4, memory_mib: 32, parallelism: 2 }, SDK_ARGON2_KEY ; "argon2id")]
fn master_key_matches_bitwarden_sdk(kdf: &Kdf, expected: [u8; 32]) {
    assert_eq!(*master_key(SDK_PASSWORD, SDK_SALT, kdf).unwrap(), expected);
}

#[test]
fn master_key_salt_is_trimmed_lowercase_email() {
    let kdf = Kdf::Pbkdf2 { iterations: 10_000 };
    let upper = format!(" {} ", SDK_SALT.to_uppercase());

    assert_eq!(
        *master_key(SDK_PASSWORD, &upper, &kdf).unwrap(),
        SDK_PBKDF2_KEY
    );
}

#[test]
fn stretch_matches_bitwarden_sdk() {
    let key = stretch(&SDK_PBKDF2_KEY).unwrap();

    assert_eq!(key.enc(), SDK_STRETCHED_ENC);
    assert_eq!(key.mac(), SDK_STRETCHED_MAC);
}

#[test]
fn decrypt_round_trips() {
    let key = Key::from_bytes(&KEY_BYTES).unwrap();

    let plain = decrypt(&encrypt(PLAIN.as_bytes(), &key), &key).unwrap();

    assert_eq!(plain.as_slice(), PLAIN.as_bytes());
}

#[test]
fn decrypt_with_wrong_key_fails_mac() {
    let key = Key::from_bytes(&KEY_BYTES).unwrap();
    let other = Key::from_bytes(&OTHER_KEY_BYTES).unwrap();

    let result = decrypt(&encrypt(PLAIN.as_bytes(), &key), &other);

    assert!(matches!(result, Err(Error::Mac)));
}

#[test]
fn decrypt_tampered_data_fails_mac() {
    let key = Key::from_bytes(&KEY_BYTES).unwrap();
    let enc_string = encrypt(PLAIN.as_bytes(), &key);
    let mut parts: Vec<&str> = enc_string.split('|').collect();
    let tampered = STANDARD.encode([0; 32]);
    parts[1] = &tampered;

    let result = decrypt(&parts.join("|"), &key);

    assert!(matches!(result, Err(Error::Mac)));
}

#[test_case("0.AAAAAAAAAAAAAAAAAAAAAA==|AAAAAAAAAAAAAAAAAAAAAA==" ; "aes cbc without mac")]
#[test_case("7.AAAA" ; "cose")]
fn decrypt_rejects_unsupported_types(enc_string: &str) {
    let key = Key::from_bytes(&KEY_BYTES).unwrap();

    assert!(matches!(
        decrypt(enc_string, &key),
        Err(Error::Unsupported(_))
    ));
}

#[test_case("no type" ; "no type")]
#[test_case("2.AAAA|AAAA" ; "missing mac")]
#[test_case("2.AAAA|AAAA|AAAA|AAAA" ; "extra part")]
#[test_case("2.!!|AAAA|AAAA" ; "not base64")]
fn decrypt_rejects_malformed(enc_string: &str) {
    let key = Key::from_bytes(&KEY_BYTES).unwrap();

    assert!(matches!(
        decrypt(enc_string, &key),
        Err(Error::Malformed(_))
    ));
}

#[test]
fn key_requires_64_bytes() {
    assert!(matches!(
        Key::from_bytes(&[0; 32]),
        Err(Error::Malformed(_))
    ));
}
