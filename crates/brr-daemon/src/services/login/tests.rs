use std::time::Duration;

use test_case::test_case;

use super::{Field, Progress, progress, to_fill, totp_wait};

const USERNAME: Field = Field::Username;
const PASSWORD: Field = Field::Password;
const TOTP: Field = Field::Totp;

#[test_case(0, Duration::ZERO; "window start")]
#[test_case(24_999, Duration::ZERO; "just over five seconds left")]
#[test_case(25_000, Duration::ZERO; "exactly five seconds left")]
#[test_case(25_001, Duration::from_millis(4_999); "just under five seconds left waits for the next window")]
#[test_case(26_000, Duration::from_secs(4); "four seconds left")]
#[test_case(29_999, Duration::from_millis(1); "one millisecond left")]
#[test_case(56_000, Duration::from_secs(4); "second window")]
fn totp_window(now_ms: u64, expected: Duration) {
    assert_eq!(totp_wait(now_ms), expected);
}

#[test_case(&[USERNAME, PASSWORD], &[], &[USERNAME, PASSWORD]; "single page")]
#[test_case(&[PASSWORD], &[USERNAME], &[PASSWORD]; "second step")]
#[test_case(&[USERNAME, TOTP], &[USERNAME, PASSWORD], &[TOTP]; "an earlier field is not refilled")]
#[test_case(&[PASSWORD], &[PASSWORD], &[]; "nothing new")]
fn fills_only_new_fields(present: &[Field], done: &[Field], expected: &[Field]) {
    assert_eq!(to_fill(present, done), expected);
}

#[test_case(&[], &[USERNAME, PASSWORD], Progress::Done; "no field left after the password")]
#[test_case(&[USERNAME], &[USERNAME, PASSWORD], Progress::Done; "a username on a logged-in page is ignored")]
#[test_case(&[], &[USERNAME], Progress::Stuck; "no password was typed yet")]
#[test_case(&[USERNAME], &[USERNAME], Progress::Stuck; "the username again")]
#[test_case(&[PASSWORD], &[USERNAME], Progress::Advance; "password after username")]
#[test_case(&[TOTP], &[USERNAME, PASSWORD], Progress::Advance; "code after password")]
#[test_case(&[PASSWORD], &[USERNAME, PASSWORD], Progress::Stuck; "password again")]
fn progress_after_submit(present: &[Field], done: &[Field], expected: Progress) {
    assert_eq!(progress(present, done), expected);
}
