use base64::{Engine, engine::general_purpose::STANDARD};
use brr_core::Secret;
use rsa::PrivateKey;
use rsa::pkcs8::DecodePrivateKey;
use serde_json::{Value, json};
use sha1::Sha1;
use test_case::test_case;

use crate::crypto::tests::encrypt;
use crate::crypto::{Kdf, Key, master_key, stretch};
use crate::{Error, Vault};

const EMAIL: &str = " Bot@Example.org";
const LOWER_EMAIL: &str = "bot@example.org";
const MASTER_PASSWORD: &str = "test master password";
const WRONG_PASSWORD: &str = "not the master password";
const ITERATIONS: u32 = 1_000;
const USER_KEY: [u8; 64] = [1; 64];
const ITEM_KEY: [u8; 64] = [2; 64];
const ORG_KEY: [u8; 64] = [3; 64];
const ORG_ID: &str = "org-bots";
const ORG_NAME: &str = "Bots";
const PERSONAL_ID: &str = "item-personal";
const KEYED_ID: &str = "item-keyed";
const ORG_ITEM_ID: &str = "item-org";
const TRASHED_ID: &str = "item-trashed";
const NOTE_ID: &str = "item-note";
const UNKNOWN_ID: &str = "item-unknown";
const NAME: &str = "Example";
const USERNAME: &str = "bot";
const PASSWORD: &str = "hunter2-correct-horse";
const TOTP: &str = "JBSWY3DPEHPK3PXP";
const URI: &str = "https://example.org/login";
const RSA_OAEP_SHA1_PREFIX: &str = "4.";
// Throwaway 1024-bit PKCS#8 test key; Bitwarden uses 2048-bit keys, the format is the same.
const PRIVATE_KEY: &str = concat!(
    "MIICdgIBADANBgkqhkiG9w0BAQEFAASCAmAwggJcAgEAAoGBAO0JHJeJTKaK+sn+T2rF44i7v48c",
    "ND49jjoqXGsmalmdgVLM7UnZ+gF8cqbr+2IE2w1MJD8/qg1sbPwqngtkG90sBo/lgc3swBRtz7IY",
    "Lhv91n5eaaS6cAk6RIx8wxrGEGEjurdHYc2NrZXTaHKxZyoNnCM85wqyMWuvBJcYL/tXAgMBAAEC",
    "gYBuPZ6em6yaiGi7KRumfQKlxSwCUwu5JsfPJYR2ERfFNgu/hq+VeVtc9ZQdLgNsQvXd6VuyTtm4",
    "BIVrgiafBaAvB9kLvThqhOmpmsoX/JZtNrxIB91PspCDLgxHXKtkK2pmFxk7O9tDJ4iGVjqmWOCo",
    "8RGOUHdDzj2u39GPo+YvgQJBAP59o9P7CcejmiznbT1cQcGjsjkMIPRy0iXqSHgeNqnXQAdtNT43",
    "Pmx+f+MocQH9aVZytAjcfiNQ+QWUF6psLeMCQQDucPjIQLACiiWOAx9ORKr0wt3Zhz6RLL0SV2Xx",
    "Iqqr5A1X+X+1ZBVa3uB25MoFXAfKJ/YbZvgEt+NuLDxTYHb9AkApr4zZWUKV1/+WIWZsaaM/w7+a",
    "xpjTUceIfGLYgiJ0GZUcYlAZW9U20lXCcMpqCAdsKKfC19M1l7ripgpmWQf/AkEAls5o6u7O46xQ",
    "OFONN0mC/q9I9U3uqo8rPjB8z1hnrbPdJKkutqEISKLyQQBPLbiZTKidpZMohoNvmrzroqg+WQJA",
    "RY3tjcA/jR2T2OzQ49+kAzXq++YfcFkdCDwd3tRR1HhIfkGiZuiny24p01huZY48gOXF1hy9zbGf",
    "N1bX0khwPA==",
);

fn token(kdf: Value) -> Vec<u8> {
    let mut token = json!({ "access_token": "access", "token_type": "Bearer" });
    for (field, value) in kdf.as_object().unwrap() {
        token[field] = value.clone();
    }
    token.to_string().into_bytes()
}

fn pbkdf2_token() -> Vec<u8> {
    token(
        json!({ "Kdf": 0, "KdfIterations": ITERATIONS, "KdfMemory": null, "KdfParallelism": null }),
    )
}

fn cipher(id: &str, organization: Option<&str>, base: &Key, item_key: Option<&[u8; 64]>) -> Value {
    let item = item_key.map(|bytes| Key::from_bytes(bytes).unwrap());
    let key = item.as_ref().unwrap_or(base);
    json!({
        "id": id,
        "organizationId": organization,
        "type": 1,
        "name": encrypt(NAME.as_bytes(), key),
        "key": item_key.map(|bytes| encrypt(bytes, base)),
        "login": {
            "username": encrypt(USERNAME.as_bytes(), key),
            "password": encrypt(PASSWORD.as_bytes(), key),
            "totp": encrypt(TOTP.as_bytes(), key),
            "uris": [{ "uri": encrypt(URI.as_bytes(), key), "match": null }, { "uri": null }],
        },
        "deletedDate": null,
    })
}

fn sync() -> Vec<u8> {
    let kdf = Kdf::Pbkdf2 {
        iterations: ITERATIONS,
    };
    let stretched = stretch(&master_key(MASTER_PASSWORD, EMAIL, &kdf).unwrap()).unwrap();
    let user_key = Key::from_bytes(&USER_KEY).unwrap();
    let org_key = Key::from_bytes(&ORG_KEY).unwrap();
    let der = STANDARD.decode(PRIVATE_KEY).unwrap();
    let public_key = PrivateKey::from_pkcs8_der(&der).unwrap().to_public_key();
    let org_key_enc = public_key.oaep_encrypt::<Sha1>(&ORG_KEY).unwrap();

    let mut trashed = cipher(TRASHED_ID, None, &user_key, None);
    trashed["deletedDate"] = json!("2026-11-01T00:00:00Z");

    json!({
        "profile": {
            "email": EMAIL,
            "key": encrypt(&USER_KEY, &stretched),
            "privateKey": encrypt(&der, &user_key),
            "organizations": [
                {
                    "id": ORG_ID,
                    "name": ORG_NAME,
                    "key": format!("{RSA_OAEP_SHA1_PREFIX}{}", STANDARD.encode(org_key_enc)),
                },
                { "id": "org-invited", "name": "Invited", "key": null },
            ],
        },
        "ciphers": [
            cipher(PERSONAL_ID, None, &user_key, None),
            cipher(KEYED_ID, None, &user_key, Some(&ITEM_KEY)),
            cipher(ORG_ITEM_ID, Some(ORG_ID), &org_key, Some(&ITEM_KEY)),
            trashed,
            { "id": NOTE_ID, "type": 2, "name": encrypt(NAME.as_bytes(), &user_key), "login": null },
        ],
    })
    .to_string()
    .into_bytes()
}

fn vault() -> Vault {
    Vault::unlock(&pbkdf2_token(), &sync(), &Secret::new(MASTER_PASSWORD)).unwrap()
}

#[test]
fn unlock_reads_lowercase_email() {
    assert_eq!(vault().email(), LOWER_EMAIL);
}

#[test]
fn items_list_only_live_logins() {
    let mut ids: Vec<String> = vault().items().into_iter().map(|item| item.id).collect();
    ids.sort();

    assert_eq!(ids, [KEYED_ID, ORG_ITEM_ID, PERSONAL_ID]);
}

#[test_case(PERSONAL_ID, None ; "personal")]
#[test_case(KEYED_ID, None ; "personal with item key")]
#[test_case(ORG_ITEM_ID, Some(ORG_NAME) ; "organization with item key")]
fn items_decrypt_metadata(id: &str, organization: Option<&str>) {
    let items = vault().items();
    let item = items.iter().find(|item| item.id == id).unwrap();

    assert_eq!(item.name, NAME);
    assert_eq!(item.username.as_deref(), Some(USERNAME));
    assert_eq!(item.uris, [URI]);
    assert!(item.has_totp);
    assert_eq!(item.organization.as_deref(), organization);
}

#[test_case(PERSONAL_ID ; "personal")]
#[test_case(KEYED_ID ; "personal with item key")]
#[test_case(ORG_ITEM_ID ; "organization with item key")]
fn login_decrypts_secrets(id: &str) {
    let login = vault().login(id).unwrap();

    assert_eq!(login.username.expose(), USERNAME);
    assert_eq!(login.password.expose(), PASSWORD);
    assert_eq!(login.totp.as_ref().map(Secret::expose), Some(TOTP));
    assert_eq!(login.uris, [URI]);
}

#[test_case(TRASHED_ID ; "trashed")]
#[test_case(NOTE_ID ; "not a login")]
#[test_case(UNKNOWN_ID ; "unknown")]
fn login_rejects_missing_items(id: &str) {
    assert!(matches!(vault().login(id), Err(Error::NotFound(found)) if found == id));
}

#[test]
fn unlock_with_wrong_password_fails() {
    let result = Vault::unlock(&pbkdf2_token(), &sync(), &Secret::new(WRONG_PASSWORD));

    assert!(matches!(result, Err(Error::WrongPassword)));
}

#[test_case(json!({ "Kdf": 7, "KdfIterations": 3 }), "unsupported" ; "unknown kdf")]
#[test_case(json!({ "Kdf": 1, "KdfIterations": 3, "KdfMemory": null, "KdfParallelism": 4 }), "protocol" ; "argon2id without memory")]
#[test_case(json!({ "Kdf": 0 }), "protocol" ; "missing iterations")]
fn unlock_rejects_bad_kdf(kdf: Value, kind: &str) {
    let result = Vault::unlock(&token(kdf), &sync(), &Secret::new(MASTER_PASSWORD));

    let actual = match result {
        Err(Error::Unsupported(_)) => "unsupported",
        Err(Error::Protocol(_)) => "protocol",
        _ => "other",
    };
    assert_eq!(actual, kind);
}

#[test]
fn vault_debug_shows_no_keys_or_secrets() {
    assert_eq!(
        format!("{:?}", vault()),
        format!("Vault {{ email: {LOWER_EMAIL:?}, items: 3, .. }}")
    );
}
