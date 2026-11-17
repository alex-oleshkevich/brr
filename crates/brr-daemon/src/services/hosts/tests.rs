use std::collections::HashMap;

use brr_core::{Error, Host, HostId, HostStatus};
use test_case::test_case;

use super::{SshTarget, capacity, fresh, parse_docker_url, sanitize};

const DESTINATION: &str = "alex@ssh.example.com";
const PORT: u16 = 2222;
const NOW_MS: i64 = 1_800_000_000_000;

#[test_case("ssh://alex@ssh.example.com", None; "default port")]
#[test_case("ssh://alex@ssh.example.com:2222", Some(PORT); "explicit port")]
fn docker_url_parses(url: &str, port: Option<u16>) {
    assert_eq!(
        parse_docker_url(url).ok(),
        Some(SshTarget {
            destination: DESTINATION,
            port
        })
    );
}

#[test_case("tcp://alex@ssh.example.com:2375"; "other scheme")]
#[test_case("ssh://ssh.example.com"; "no user")]
#[test_case("ssh://alex@"; "no host")]
#[test_case("ssh://-oProxyCommand=x@host"; "option as user")]
#[test_case("ssh://alex@-oProxyCommand=x"; "option as host")]
#[test_case("ssh://alex@host/path"; "path")]
#[test_case("ssh://alex@host:0"; "port zero")]
#[test_case("ssh://alex@host:ssh"; "named port")]
#[test_case("ssh://alex@host name"; "space")]
fn docker_url_is_refused(url: &str) {
    assert!(matches!(
        parse_docker_url(url),
        Err(Error::InvalidArgument { .. })
    ));
}

#[test]
fn sanitize_drops_control_characters() {
    assert_eq!(sanitize("\u{1b}[31mdenied\r\n"), "[31mdenied");
}

#[test_case(None, false; "never probed")]
#[test_case(Some(NOW_MS - 29_999), true; "just inside the age")]
#[test_case(Some(NOW_MS - 30_000), false; "at the age")]
#[test_case(Some(NOW_MS + 1_000), false; "checked after now: the clock went back")]
fn placement_status_age(checked_at: Option<i64>, expected: bool) {
    assert_eq!(fresh(checked_at, NOW_MS), expected);
}

const LOCAL_MAX: u32 = 1;
const HOST_MAX: u32 = 2;

fn host(reachable: bool) -> Host {
    Host {
        id: HostId::new(),
        name: "box".to_owned(),
        docker_url: "ssh://brr@box".to_owned(),
        max_instances: HOST_MAX,
        enabled: true,
        created_at: 0,
        status: HostStatus {
            reachable,
            ..HostStatus::default()
        },
    }
}

// (local running, running on the Host, Host reachable) -> at capacity
#[test_case(0, 0, true, false; "room locally")]
#[test_case(1, 1, true, false; "local full, room on the host")]
#[test_case(1, 2, true, true; "local and host full")]
#[test_case(1, 0, false, true; "local full, host unreachable")]
fn capacity_is_full_only_everywhere(local: u32, on_host: u32, reachable: bool, full: bool) {
    let found = host(reachable);
    let placed = HashMap::from([(None, local), (Some(found.id), on_host)]);

    let counts = capacity(&placed, LOCAL_MAX, &[found]);

    assert_eq!(counts.local_running, local);
    assert_eq!(counts.hosts_running, on_host);
    assert_eq!(counts.hosts_max, if reachable { HOST_MAX } else { 0 });
    assert_eq!(counts.at_capacity, full);
}
