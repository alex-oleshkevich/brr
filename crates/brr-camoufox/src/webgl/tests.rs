use test_case::test_case;

use super::{gpu_screen_is_plausible, is_software_renderer, renderer_bucket};

const NVIDIA_980: &str =
    "ANGLE (NVIDIA, NVIDIA GeForce GTX 980 Direct3D11 vs_5_0 ps_5_0), or similar";

#[test_case(NVIDIA_980, "GeForce GTX 980"; "angle direct3d")]
#[test_case("ANGLE (Samsung Xclipse 920) on Vulkan", "Samsung Xclipse 920"; "angle vulkan")]
#[test_case("NVIDIA GeForce GTX 980/PCIe/SSE2", "GeForce GTX 980"; "pcie")]
#[test_case("GeForce GTX 480/PCI/SSE2", "GeForce GTX 480"; "pci")]
#[test_case("Radeon HD 3200 Graphics, or similar", "Radeon HD 3200 Graphics"; "plain")]
fn bucket_reduces_the_renderer(renderer: &str, expected: &str) {
    assert_eq!(renderer_bucket(renderer), expected);
}

#[test_case(Some(NVIDIA_980), Some(1024), Some(600), false; "discrete gpu on a netbook panel")]
#[test_case(Some(NVIDIA_980), Some(1920), Some(1080), true; "discrete gpu on a desktop panel")]
#[test_case(Some(NVIDIA_980), None, None, true; "unknown screen")]
#[test_case(Some(NVIDIA_980), Some(0), Some(600), true; "zero width")]
#[test_case(Some("llvmpipe, or similar"), Some(800), Some(480), true; "software renderer")]
fn gpu_screen_plausibility(
    renderer: Option<&str>,
    width: Option<i64>,
    height: Option<i64>,
    expected: bool,
) {
    assert_eq!(gpu_screen_is_plausible(renderer, width, height), expected);
}

#[test]
fn software_renderers_are_recognized() {
    assert!(is_software_renderer(Some(
        "ANGLE (Google, SwiftShader driver)"
    )));
    assert!(!is_software_renderer(Some(NVIDIA_980)));
    assert!(!is_software_renderer(None));
}
