use std::path::PathBuf;

use brr_core::InstanceId;
use brr_engine::sandbox::{CONTAINER_PREFIX, MountRoot, MountSource};
use test_case::test_case;

use super::{parse_inspect, parse_mem_usage, parse_stats};

const IMAGE: &str = "ghcr.io/example/brr:0.4.0";
const VOLUME: &str = "brr-data";
const VOLUME_AT: &str = "/data";
const BIND_SOURCE: &str = "/home/alex/downloads";
const BIND_AT: &str = "/downloads";
const MIB: u64 = 1024 * 1024;
const GIB: u64 = 1024 * MIB;

fn inspect_output(image: &str) -> String {
    format!(
        r#"[{{
            "Id": "4f1c2a",
            "Config": {{"Hostname": "4f1c2a", "Image": "{image}"}},
            "Mounts": [
                {{"Type": "volume", "Name": "{VOLUME}", "Source": "/var/lib/docker/volumes/{VOLUME}/_data",
                  "Destination": "{VOLUME_AT}", "Driver": "local", "Mode": "z", "RW": true, "Propagation": ""}},
                {{"Type": "bind", "Source": "{BIND_SOURCE}", "Destination": "{BIND_AT}",
                  "Mode": "", "RW": true, "Propagation": "rprivate"}},
                {{"Type": "tmpfs", "Source": "", "Destination": "/tmp", "Mode": "", "RW": true, "Propagation": ""}}
            ]
        }}]"#
    )
}

#[test]
fn discovery_takes_volumes_and_binds() {
    let (image, roots) = parse_inspect(inspect_output(IMAGE).as_bytes()).unwrap();

    assert_eq!(image.as_deref(), Some(IMAGE));
    assert_eq!(
        roots,
        vec![
            MountRoot {
                path: PathBuf::from(VOLUME_AT),
                source: MountSource::Volume(VOLUME.to_owned()),
            },
            MountRoot {
                path: PathBuf::from(BIND_AT),
                source: MountSource::Bind(PathBuf::from(BIND_SOURCE)),
            },
        ]
    );
}

#[test]
fn discovery_without_image() {
    let (image, _) = parse_inspect(inspect_output("").as_bytes()).unwrap();

    assert_eq!(image, None);
}

#[test]
fn discovery_of_nothing_fails() {
    assert!(parse_inspect(b"[]").is_err());
}

#[test_case("512MiB / 2GiB", Some(512 * MIB); "mebibytes")]
#[test_case("1.5GiB / 2GiB", Some(GIB + 512 * MIB); "fractional gibibytes")]
#[test_case("300KiB / 1GiB", Some(300 * 1024); "kibibytes")]
#[test_case("12.5MB / 1GB", Some(12_500_000); "decimal megabytes")]
#[test_case("0B / 0B", Some(0); "nothing")]
#[test_case("-- / --", None; "stopped container")]
fn mem_usage(text: &str, expected: Option<u64>) {
    assert_eq!(parse_mem_usage(text), expected);
}

#[test]
fn stats_keep_only_brr_containers() {
    let instance_id = InstanceId::new();
    let output = format!(
        "{{\"Name\":\"{CONTAINER_PREFIX}{instance_id}\",\"MemUsage\":\"512MiB / 2GiB\",\"CPUPerc\":\"1.00%\"}}\n\
         {{\"Name\":\"postgres\",\"MemUsage\":\"1GiB / 4GiB\"}}\n"
    );

    let rss = parse_stats(output.as_bytes());

    assert_eq!(rss.len(), 1);
    assert_eq!(rss.get(&instance_id), Some(&(512 * MIB)));
}
