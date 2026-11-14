use std::time::Duration;

use brr_core::{InstanceId, ProfileId};
use tokio::time::{self, Instant};

use super::{POOL_BACKOFF_BASE, POOL_BACKOFF_MAX, POOL_STABLE, Pool};

const ROOM: usize = 20;

fn open_everywhere(_: &InstanceId) -> Option<usize> {
    Some(1)
}

fn pool_of(profile_id: ProfileId, contexts: &[usize]) -> (Pool, Vec<InstanceId>) {
    let mut pool = Pool::default();
    let ids: Vec<InstanceId> = contexts.iter().map(|_| InstanceId::new()).collect();
    for (instance_id, count) in ids.iter().zip(contexts) {
        pool.track(*instance_id, profile_id, *count);
    }

    (pool, ids)
}

#[test]
fn reserve_picks_the_member_with_the_fewest_contexts_and_counts_it() {
    let profile_id = ProfileId::new();
    let (mut pool, ids) = pool_of(profile_id, &[2, 0, 1]);

    let picked = pool.reserve(&profile_id, ROOM, open_everywhere);

    assert_eq!(picked, Some(ids[1]));
    assert_eq!(pool.members[&ids[1]].contexts, 1);
}

#[test]
fn reserve_skips_other_profiles_and_members_without_room() {
    let profile_id = ProfileId::new();
    let (mut pool, ids) = pool_of(profile_id, &[0]);

    assert_eq!(pool.reserve(&ProfileId::new(), ROOM, open_everywhere), None);
    assert_eq!(pool.reserve(&profile_id, ROOM, |_| None), None);
    // Contexts still opening count against the limit before their tabs are listed.
    assert_eq!(pool.reserve(&profile_id, 1, open_everywhere), None);
    assert_eq!(
        pool.reserve(&profile_id, ROOM, open_everywhere),
        Some(ids[0])
    );
}

#[test]
fn release_reports_the_last_context() {
    let profile_id = ProfileId::new();
    let (mut pool, ids) = pool_of(profile_id, &[2]);

    assert!(!pool.release(&ids[0]));
    assert!(pool.release(&ids[0]));
    // Releasing again, or an unknown Instance, never underflows.
    assert!(pool.release(&ids[0]));
    assert!(!pool.release(&InstanceId::new()));
}

#[test]
fn expire_keeps_pool_members_up_to_the_pool_size() {
    let profile_id = ProfileId::new();
    let (mut pool, ids) = pool_of(profile_id, &[0, 0]);

    assert!(!pool.expire(&ids[0], Some(&profile_id), 2));
    assert!(pool.expire(&ids[0], Some(&profile_id), 1));
    assert!(!pool.expire(&ids[1], Some(&profile_id), 1));
    assert_eq!(pool.count(&profile_id), 1);
}

#[test]
fn expire_never_takes_a_member_with_contexts_and_always_takes_foreign_ones() {
    let (pool_profile, other) = (ProfileId::new(), ProfileId::new());
    let (mut pool, ids) = pool_of(pool_profile, &[1]);
    pool.track(InstanceId::new(), other, 0);
    let foreign = *pool.members.keys().find(|id| **id != ids[0]).unwrap();

    assert!(!pool.expire(&ids[0], Some(&pool_profile), 0));
    // The pool is empty of its own members, yet the other Profile's Instance does not count.
    assert!(pool.expire(&foreign, Some(&pool_profile), 5));
    assert_eq!(pool.count(&pool_profile), 1);
}

#[test]
fn trim_removes_only_idle_surplus_members() {
    let profile_id = ProfileId::new();
    let (mut pool, ids) = pool_of(profile_id, &[0, 1, 0]);

    let trimmed = pool.trim(&profile_id, 1);

    assert_eq!(trimmed.len(), 2);
    assert!(!trimmed.contains(&ids[1]));
    assert_eq!(pool.count(&profile_id), 1);
}

#[tokio::test]
async fn reserve_cancels_the_linger_timer() {
    let profile_id = ProfileId::new();
    let (mut pool, ids) = pool_of(profile_id, &[0]);
    let timer = tokio::spawn(time::sleep(Duration::from_secs(3600)));
    pool.set_linger(&ids[0], timer.abort_handle());

    pool.reserve(&profile_id, ROOM, open_everywhere);

    assert!(timer.await.unwrap_err().is_cancelled());
}

#[test]
fn retain_alive_drops_members_whose_instance_is_gone() {
    let profile_id = ProfileId::new();
    let (mut pool, ids) = pool_of(profile_id, &[0, 0, 0]);

    pool.retain_alive(|instance_id| *instance_id != ids[1]);

    assert_eq!(pool.count(&profile_id), 2);
    assert!(!pool.members.contains_key(&ids[1]));
}

#[test]
fn a_crash_soon_after_the_start_delays_the_refill_and_doubles() {
    let profile_id = ProfileId::new();
    let (mut pool, ids) = pool_of(profile_id, &[0, 0, 0]);
    let now = Instant::now();

    assert!(pool.forget(&ids[0], true, now));
    assert_eq!(pool.refill_at, Some(now + POOL_BACKOFF_BASE));
    assert_eq!(pool.backing_off(now), Some(now + POOL_BACKOFF_BASE));
    assert_eq!(pool.backing_off(now + POOL_BACKOFF_BASE), None);

    assert!(pool.forget(&ids[1], true, now));
    assert_eq!(pool.refill_at, Some(now + POOL_BACKOFF_BASE * 2));
}

#[test]
fn backoff_is_capped_and_a_clean_end_or_a_long_run_resets_it() {
    let profile_id = ProfileId::new();
    let (mut pool, ids) = pool_of(profile_id, &[0, 0]);
    let now = Instant::now();
    pool.crashes = 40;

    pool.forget(&ids[0], true, now);
    assert_eq!(pool.refill_at, Some(now + POOL_BACKOFF_MAX));

    pool.forget(&ids[1], false, now);
    assert_eq!(pool.crashes, 0);

    let late = now + POOL_STABLE;
    let (mut pool, ids) = pool_of(profile_id, &[0]);
    pool.crashes = 3;
    pool.forget(&ids[0], true, late + POOL_STABLE);
    assert_eq!(pool.crashes, 0);
    assert_eq!(pool.refill_at, None);
}
