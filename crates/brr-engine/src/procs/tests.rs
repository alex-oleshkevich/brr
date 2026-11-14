use brr_core::InstanceId;
use test_case::test_case;

use super::parse_environ;

#[test]
fn parse_environ_finds_the_marker() {
    let id = InstanceId::new();
    let environ = format!("PATH=/usr/bin\0BRR_INSTANCE_ID={id}\0BRR_DAEMON=1\0");

    assert_eq!(parse_environ(environ.as_bytes()), Some(id));
}

#[test_case(b"PATH=/usr/bin\0BRR_DAEMON=1\0"; "no marker")]
#[test_case(b"BRR_INSTANCE_ID=not-a-ulid\0"; "malformed id")]
#[test_case(b"XBRR_INSTANCE_ID=01ARZ3NDEKTSV4RRFFQ69G5FAV\0"; "prefix glued to another name")]
#[test_case(b""; "empty environ")]
fn parse_environ_rejects(environ: &[u8]) {
    assert_eq!(parse_environ(environ), None);
}

fn write_children(proc_dir: &std::path::Path, pid: u32, tid: u32, children: &str) {
    let task = proc_dir
        .join(pid.to_string())
        .join("task")
        .join(tid.to_string());
    std::fs::create_dir_all(&task).unwrap();
    std::fs::write(task.join("children"), children).unwrap();
}

#[test]
fn descendants_walks_every_thread_and_level() {
    let root = tempfile::tempdir().unwrap();
    write_children(root.path(), 10, 10, "11 12 ");
    write_children(root.path(), 10, 13, "14 ");
    write_children(root.path(), 11, 11, "15 ");
    write_children(root.path(), 99, 99, "100 ");

    let mut found = super::descendants_in(root.path(), 10);
    found.sort_unstable();

    assert_eq!(found, [10, 11, 12, 14, 15]);
}

#[test]
fn descendants_of_a_gone_process_is_the_pid_alone() {
    let root = tempfile::tempdir().unwrap();

    assert_eq!(super::descendants_in(root.path(), 7), [7]);
}
