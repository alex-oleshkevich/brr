use brr_core::{EventData, InstanceId, Order, Page, ProfileId, TabId};
use brr_store::{Store, event};

use super::EventBus;

fn profile_created() -> EventData {
    EventData::ProfileCreated {
        profile_id: ProfileId::new(),
        name: "work".to_string(),
    }
}

fn tab_navigated() -> EventData {
    EventData::TabNavigated {
        tab_id: TabId::new(),
        instance_id: InstanceId::new(),
        url: "https://example.com".to_string(),
        title: "Example".to_string(),
    }
}

async fn persisted(store: &Store) -> Vec<i64> {
    let query = event::Query {
        since: None,
        kinds: Vec::new(),
        subject: None,
        instance_id: None,
        from_ts: None,
        to_ts: None,
        order: Order::Asc,
        page: Page::new(None, None),
    };
    let (events, _) = store.run(move |c| event::list(c, &query)).await.unwrap();

    events.iter().map(|e| e.seq).collect()
}

#[tokio::test]
async fn transient_events_are_broadcast_but_not_stored() {
    let store = Store::open_in_memory().unwrap();
    let bus = EventBus::new(store.clone()).await.unwrap();
    let mut rx = bus.subscribe();

    bus.publish(profile_created()).await.unwrap();
    bus.publish(tab_navigated()).await.unwrap();
    bus.publish(profile_created()).await.unwrap();

    let mut delivered = Vec::new();
    for _ in 0..3 {
        delivered.push(rx.recv().await.unwrap().event.seq);
    }
    assert_eq!(delivered, [1, 2, 3]);
    assert_eq!(persisted(&store).await, [1, 3]);
}

#[tokio::test]
async fn subscribers_see_seq_order_under_concurrent_publishes() {
    let store = Store::open_in_memory().unwrap();
    let bus = std::sync::Arc::new(EventBus::new(store).await.unwrap());
    let mut rx = bus.subscribe();

    let mut tasks = Vec::new();
    for _ in 0..20 {
        let bus = std::sync::Arc::clone(&bus);
        tasks.push(tokio::spawn(async move {
            bus.publish(profile_created()).await.unwrap()
        }));
    }
    for task in tasks {
        task.await.unwrap();
    }

    let mut seqs = Vec::new();
    for _ in 0..20 {
        seqs.push(rx.recv().await.unwrap().event.seq);
    }
    assert_eq!(seqs, (1..=20).collect::<Vec<i64>>());
}

#[tokio::test]
async fn seq_continues_after_restart() {
    let store = Store::open_in_memory().unwrap();
    let first = EventBus::new(store.clone()).await.unwrap();
    first.publish(profile_created()).await.unwrap();
    first.publish(profile_created()).await.unwrap();

    let second = EventBus::new(store).await.unwrap();
    let published = second.publish(profile_created()).await.unwrap();

    assert_eq!(published.event.seq, 3);
    assert_eq!(second.last_seq().await, 3);
}

#[tokio::test]
async fn seq_stays_ahead_of_transient_events_after_restart() {
    let store = Store::open_in_memory().unwrap();
    let first = EventBus::new(store.clone()).await.unwrap();
    first.publish(profile_created()).await.unwrap();

    let mut last_seen = 0;
    for _ in 0..3 {
        last_seen = first.publish(tab_navigated()).await.unwrap().event.seq;
    }

    let second = EventBus::new(store.clone()).await.unwrap();
    assert!(second.boot_seq() >= last_seen);
    assert!(second.publish(profile_created()).await.unwrap().event.seq > last_seen);

    store
        .run(|c| brr_store::retention::delete_events_before(c, i64::MAX))
        .await
        .unwrap();
    let third = EventBus::new(store).await.unwrap();

    assert!(third.publish(profile_created()).await.unwrap().event.seq > last_seen);
}
