use brr_core::Size;
use brr_core::capture::{PAPER_SIZES, PdfOptions};
use serde_json::json;

use super::{DEFAULT_MARGIN_IN, pdf_params};
use crate::page::devices::device;

#[test]
fn letter_landscape_with_inch_margins() {
    let options = PdfOptions {
        paper: Some("Letter".to_owned()),
        landscape: Some(true),
        margin: Some("1in".to_owned()),
        ..PdfOptions::default()
    };

    let params = pdf_params(&options).unwrap();

    assert_eq!(params["paperWidth"], json!(11.0));
    assert_eq!(params["paperHeight"], json!(8.5));
    for side in ["marginTop", "marginRight", "marginBottom", "marginLeft"] {
        assert_eq!(params[side], json!(1.0), "{side}");
    }
}

#[test]
fn pdf_defaults_are_a4_with_ten_millimeter_margins() {
    let params = pdf_params(&PdfOptions::default()).unwrap();

    let (_, width, height) = PAPER_SIZES.iter().find(|(name, ..)| *name == "A4").unwrap();
    assert_eq!(params["paperWidth"], json!(width));
    assert_eq!(params["paperHeight"], json!(height));
    assert_eq!(params["marginTop"], json!(DEFAULT_MARGIN_IN));
    assert_eq!(params["scale"], json!(1.0));
    assert_eq!(params["pageRanges"], json!(""));
    assert_eq!(params["transferMode"], json!("ReturnAsStream"));
}

#[test]
fn devices_come_from_the_vendored_descriptors() {
    let phone = device("iPhone 15").unwrap();

    assert_eq!(
        phone.viewport,
        Size {
            width: 393,
            height: 659
        }
    );
    assert!((phone.scale - 3.0).abs() < f64::EPSILON);
    assert!(phone.mobile && phone.touch);
    assert!(device("Nope").is_none());
}
