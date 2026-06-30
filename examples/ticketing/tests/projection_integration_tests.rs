//! Integration tests for Projectors and Projection Queries.
//!
//! Single container, all tests inline.
//!
//! ```bash
//! cargo test -p ticketing --test projection_integration_tests
//! ```

#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::too_many_lines)]

use chrono::{Duration, Utc};
use composable_rust_auth::state::UserId;
use composable_rust_next::{Projector, SerializedEvent};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;
use ticketing::next::{
    EventEvent, EventProjector, EventProjectionQueries, InventoryEvent, InventoryProjector,
    InventoryProjectionQueries, PaymentEvent, PaymentProjector, PaymentProjectionQueries,
    ReleaseReason,
};
use ticketing::types::{
    Capacity, CustomerId, EventDate, EventId, EventStatus, Money, PaymentId, PaymentMethod,
    PricingTier, ReservationId, SeatId, SeatType, TierType, Venue, VenueSection,
};

fn venue() -> Venue {
    Venue {
        name: "Arena".to_string(),
        capacity: Capacity::new(200),
        sections: vec![
            VenueSection {
                name: "VIP".to_string(),
                capacity: Capacity::new(50),
                seat_type: SeatType::GeneralAdmission,
            },
            VenueSection {
                name: "General".to_string(),
                capacity: Capacity::new(150),
                seat_type: SeatType::GeneralAdmission,
            },
        ],
    }
}

fn pricing() -> Vec<PricingTier> {
    vec![PricingTier {
        tier_type: TierType::Regular,
        section: "VIP".to_string(),
        base_price: Money::from_cents(10000),
        available_from: Utc::now(),
        available_until: None,
    }]
}

fn ser<T: serde::Serialize>(event: &T, typ: &str) -> SerializedEvent {
    SerializedEvent {
        event_type: typ.to_string(),
        payload: bincode::serialize(event).expect("serialize"),
        metadata: None,
        version: None,
    }
}

async fn reset(pool: &PgPool) {
    sqlx::query(
        "TRUNCATE TABLE events_projection, inventory, reservations, reservations_projection,
         payments, sales_analytics_projection, sales_analytics_sections,
         customer_profiles, customer_event_attendance, customer_purchases, seats
         RESTART IDENTITY CASCADE",
    )
    .execute(pool)
    .await
    .expect("truncate");
}

#[tokio::test]
async fn all_projection_tests() {
    // ═══════════════════════════════════════════════════════════════════════
    // Setup
    // ═══════════════════════════════════════════════════════════════════════
    let container = Postgres::default().start().await.expect("start");
    let port = container.get_host_port_ipv4(5432).await.expect("port");
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");

    let pool = loop {
        if let Ok(p) = PgPoolOptions::new().connect(&url).await {
            if sqlx::query("SELECT 1").execute(&p).await.is_ok() {
                break p;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    };

    // Single-DB merged migrations (same schema the application runs).
    for sql in [
        include_str!("../migrations/001_events_log.sql"),
        include_str!("../migrations/002_projections.sql"),
        include_str!("../migrations/003_seats.sql"),
        include_str!("../migrations/004_saga_state.sql"),
    ] {
        sqlx::raw_sql(sql).execute(&pool).await.expect("migration");
    }

    let now = Utc::now();

    // ═══════════════════════════════════════════════════════════════════════
    // EventProjector - All Event Types
    // ═══════════════════════════════════════════════════════════════════════
    let ep = EventProjector::new(pool.clone());
    let eq = EventProjectionQueries::new(pool.clone());

    // 1. Created
    reset(&pool).await;
    let eid = EventId::new();
    let owner = UserId::new();
    ep.project(&[ser(&EventEvent::Created {
        event_id: eid, name: "Concert".into(), owner_id: owner, venue: venue(),
        date: EventDate::new(now + Duration::days(30)), pricing_tiers: pricing(), created_at: now,
    }, "EventCreated")]).await.expect("project");
    let dto = eq.get_event(eid).await.expect("q").unwrap();
    assert_eq!(dto.name, "Concert");
    assert_eq!(dto.status, EventStatus::Draft);
    println!("  ✓ EventProjector: Created");

    // 2. Updated (name only)
    ep.project(&[ser(&EventEvent::Updated {
        event_id: eid, name: Some("New Name".into()), venue: None, date: None, updated_at: now,
    }, "EventUpdated")]).await.expect("project");
    assert_eq!(eq.get_event(eid).await.unwrap().unwrap().name, "New Name");
    println!("  ✓ EventProjector: Updated name");

    // 3. Updated (date only)
    let new_date = now + Duration::days(60);
    ep.project(&[ser(&EventEvent::Updated {
        event_id: eid, name: None, venue: None, date: Some(EventDate::new(new_date)), updated_at: now,
    }, "EventUpdated")]).await.expect("project");
    assert_eq!(eq.get_event(eid).await.unwrap().unwrap().date.inner(), new_date);
    println!("  ✓ EventProjector: Updated date");

    // 4. Published
    ep.project(&[ser(&EventEvent::Published { event_id: eid, published_at: now }, "EventPublished")])
        .await.expect("project");
    assert_eq!(eq.get_event(eid).await.unwrap().unwrap().status, EventStatus::Published);
    println!("  ✓ EventProjector: Published");

    // 5. Cancelled
    reset(&pool).await;
    let eid2 = EventId::new();
    ep.project(&[ser(&EventEvent::Created {
        event_id: eid2, name: "ToCancel".into(), owner_id: owner, venue: venue(),
        date: EventDate::new(now + Duration::days(30)), pricing_tiers: pricing(), created_at: now,
    }, "EventCreated")]).await.expect("project");
    ep.project(&[ser(&EventEvent::Cancelled {
        event_id: eid2, reason: "Artist sick".into(), cancelled_at: now,
    }, "EventCancelled")]).await.expect("project");
    assert_eq!(eq.get_event(eid2).await.unwrap().unwrap().status, EventStatus::Cancelled);
    println!("  ✓ EventProjector: Cancelled");

    // 6. PricingUpdated
    reset(&pool).await;
    let eid3 = EventId::new();
    ep.project(&[ser(&EventEvent::Created {
        event_id: eid3, name: "PriceTest".into(), owner_id: owner, venue: venue(),
        date: EventDate::new(now + Duration::days(30)), pricing_tiers: pricing(), created_at: now,
    }, "EventCreated")]).await.expect("project");
    let new_pricing = vec![PricingTier {
        tier_type: TierType::Regular, section: "VIP".into(),
        base_price: Money::from_cents(20000), available_from: now, available_until: None,
    }];
    ep.project(&[ser(&EventEvent::PricingUpdated {
        event_id: eid3, pricing_tiers: new_pricing.clone(), updated_at: now,
    }, "EventPricingUpdated")]).await.expect("project");
    let dto = eq.get_event(eid3).await.unwrap().unwrap();
    assert_eq!(dto.pricing_tiers[0].base_price.cents(), 20000);
    println!("  ✓ EventProjector: PricingUpdated");

    // 7. VenueSectionsAdded
    let new_section = VenueSection { name: "Balcony".into(), capacity: Capacity::new(100), seat_type: SeatType::GeneralAdmission };
    ep.project(&[ser(&EventEvent::VenueSectionsAdded {
        event_id: eid3, sections: vec![new_section], added_at: now,
    }, "EventVenueSectionsAdded")]).await.expect("project");
    let dto = eq.get_event(eid3).await.unwrap().unwrap();
    assert_eq!(dto.venue.sections.len(), 3);
    println!("  ✓ EventProjector: VenueSectionsAdded");

    // 8. Idempotency - duplicate create should not error
    reset(&pool).await;
    let eid4 = EventId::new();
    let create_ev = ser(&EventEvent::Created {
        event_id: eid4, name: "Idem".into(), owner_id: owner, venue: venue(),
        date: EventDate::new(now + Duration::days(30)), pricing_tiers: pricing(), created_at: now,
    }, "EventCreated");
    ep.project(std::slice::from_ref(&create_ev)).await.expect("first");
    ep.project(std::slice::from_ref(&create_ev)).await.expect("second should be idempotent");
    assert_eq!(eq.list_by_owner(owner).await.unwrap().len(), 1);
    println!("  ✓ EventProjector: Idempotent on duplicate");

    // 9. Batch projection
    reset(&pool).await;
    let batch_owner = UserId::new();
    let batch: Vec<_> = (0..5).map(|i| ser(&EventEvent::Created {
        event_id: EventId::new(), name: format!("Batch{i}"), owner_id: batch_owner, venue: venue(),
        date: EventDate::new(now + Duration::days(i.into())), pricing_tiers: pricing(), created_at: now,
    }, "EventCreated")).collect();
    ep.project(&batch).await.expect("batch");
    assert_eq!(eq.list_by_owner(batch_owner).await.unwrap().len(), 5);
    println!("  ✓ EventProjector: Batch projection");

    // ═══════════════════════════════════════════════════════════════════════
    // EventProjectionQueries - All Query Methods
    // ═══════════════════════════════════════════════════════════════════════

    // 10. get_event not found
    reset(&pool).await;
    assert!(eq.get_event(EventId::new()).await.unwrap().is_none());
    println!("  ✓ EventQueries: get_event not found");

    // 11. list_by_owner empty
    assert!(eq.list_by_owner(UserId::new()).await.unwrap().is_empty());
    println!("  ✓ EventQueries: list_by_owner empty");

    // 12. list_by_owner filters by owner
    reset(&pool).await;
    let o1 = UserId::new();
    let o2 = UserId::new();
    for i in 0..3 {
        ep.project(&[ser(&EventEvent::Created {
            event_id: EventId::new(), name: format!("O1-{i}"), owner_id: o1, venue: venue(),
            date: EventDate::new(now + Duration::days(i.into())), pricing_tiers: pricing(), created_at: now,
        }, "EventCreated")]).await.unwrap();
    }
    for i in 0..2 {
        ep.project(&[ser(&EventEvent::Created {
            event_id: EventId::new(), name: format!("O2-{i}"), owner_id: o2, venue: venue(),
            date: EventDate::new(now + Duration::days(i.into())), pricing_tiers: pricing(), created_at: now,
        }, "EventCreated")]).await.unwrap();
    }
    assert_eq!(eq.list_by_owner(o1).await.unwrap().len(), 3);
    assert_eq!(eq.list_by_owner(o2).await.unwrap().len(), 2);
    println!("  ✓ EventQueries: list_by_owner filters correctly");

    // 13. list_all pagination
    reset(&pool).await;
    let pg_owner = UserId::new();
    for i in 0..10 {
        ep.project(&[ser(&EventEvent::Created {
            event_id: EventId::new(), name: format!("Pg{i}"), owner_id: pg_owner, venue: venue(),
            date: EventDate::new(now + Duration::days(i.into())), pricing_tiers: pricing(), created_at: now,
        }, "EventCreated")]).await.unwrap();
    }
    let (page0, total) = eq.list_all(None, 0, 3).await.unwrap();
    assert_eq!(page0.len(), 3);
    assert_eq!(total, 10);
    let (page3, _) = eq.list_all(None, 3, 3).await.unwrap();
    assert_eq!(page3.len(), 1); // last page
    println!("  ✓ EventQueries: list_all pagination");

    // 14. list_all status filter
    let ids: Vec<EventId> = eq.list_by_owner(pg_owner).await.unwrap().iter().map(|e| e.id).collect();
    for id in &ids[0..3] {
        ep.project(&[ser(&EventEvent::Published { event_id: *id, published_at: now }, "EventPublished")])
            .await.unwrap();
    }
    let (drafts, draft_total) = eq.list_all(Some(EventStatus::Draft), 0, 100).await.unwrap();
    let (published, pub_total) = eq.list_all(Some(EventStatus::Published), 0, 100).await.unwrap();
    assert_eq!(draft_total, 7);
    assert_eq!(pub_total, 3);
    assert_eq!(drafts.len(), 7);
    assert_eq!(published.len(), 3);
    println!("  ✓ EventQueries: list_all status filter");

    // ═══════════════════════════════════════════════════════════════════════
    // InventoryProjector - All Event Types
    // ═══════════════════════════════════════════════════════════════════════
    let ip = InventoryProjector::new(pool.clone());
    let iq = InventoryProjectionQueries::new(pool.clone());

    // 15. Initialized
    reset(&pool).await;
    let ev_id = EventId::new();
    let seats: Vec<SeatId> = (0..50).map(|_| SeatId::new()).collect();
    ip.project(&[ser(&InventoryEvent::Initialized {
        event_id: ev_id, section: "VIP".into(), capacity: 50, seats: seats.clone(), initialized_at: now,
    }, "InventoryInitialized")]).await.expect("project");
    let avail = iq.get_section_availability(ev_id, "VIP").await.unwrap().unwrap();
    assert_eq!(avail.total_capacity, 50);
    assert_eq!(avail.available_seats, 50);
    assert_eq!(avail.reserved_seats, 0);
    println!("  ✓ InventoryProjector: Initialized");

    // 16. SeatsReserved
    let res_id = ReservationId::new();
    let reserved: Vec<SeatId> = seats[0..10].to_vec();
    ip.project(&[ser(&InventoryEvent::SeatsReserved {
        reservation_id: res_id, event_id: ev_id, section: "VIP".into(), seats: reserved.clone(),
        expires_at: now + Duration::minutes(15), reserved_at: now,
    }, "SeatsReserved")]).await.expect("project");
    let avail = iq.get_section_availability(ev_id, "VIP").await.unwrap().unwrap();
    assert_eq!(avail.available_seats, 40);
    assert_eq!(avail.reserved_seats, 10);
    println!("  ✓ InventoryProjector: SeatsReserved");

    // 17. SeatsConfirmed
    let cust = CustomerId::new();
    ip.project(&[ser(&InventoryEvent::SeatsConfirmed {
        reservation_id: res_id, customer_id: cust, seats: reserved.clone(), confirmed_at: now,
    }, "SeatsConfirmed")]).await.expect("project");
    // After confirm, reservation is no longer "active" in reservations table
    let res_dto = iq.get_reservation_dto(res_id).await.unwrap();
    assert!(res_dto.is_none()); // reservation is now confirmed, not active
    println!("  ✓ InventoryProjector: SeatsConfirmed");

    // 18. SeatsReleased (expired)
    reset(&pool).await;
    let ev_id2 = EventId::new();
    let seats2: Vec<SeatId> = (0..30).map(|_| SeatId::new()).collect();
    ip.project(&[ser(&InventoryEvent::Initialized {
        event_id: ev_id2, section: "GA".into(), capacity: 30, seats: seats2.clone(), initialized_at: now,
    }, "InventoryInitialized")]).await.unwrap();
    let res_id2 = ReservationId::new();
    let to_reserve: Vec<SeatId> = seats2[0..5].to_vec();
    ip.project(&[ser(&InventoryEvent::SeatsReserved {
        reservation_id: res_id2, event_id: ev_id2, section: "GA".into(), seats: to_reserve.clone(),
        expires_at: now + Duration::minutes(15), reserved_at: now,
    }, "SeatsReserved")]).await.unwrap();
    ip.project(&[ser(&InventoryEvent::SeatsReleased {
        reservation_id: res_id2, seats: to_reserve, reason: ReleaseReason::Expired, released_at: now,
    }, "SeatsReleased")]).await.unwrap();
    let avail = iq.get_section_availability(ev_id2, "GA").await.unwrap().unwrap();
    assert_eq!(avail.available_seats, 30);
    assert_eq!(avail.reserved_seats, 0);
    println!("  ✓ InventoryProjector: SeatsReleased (Expired)");

    // 19. SeatsReleased (customer cancelled)
    reset(&pool).await;
    let ev_id3 = EventId::new();
    let seats3: Vec<SeatId> = (0..20).map(|_| SeatId::new()).collect();
    ip.project(&[ser(&InventoryEvent::Initialized {
        event_id: ev_id3, section: "Floor".into(), capacity: 20, seats: seats3.clone(), initialized_at: now,
    }, "InventoryInitialized")]).await.unwrap();
    let res_id3 = ReservationId::new();
    ip.project(&[ser(&InventoryEvent::SeatsReserved {
        reservation_id: res_id3, event_id: ev_id3, section: "Floor".into(), seats: seats3[0..3].to_vec(),
        expires_at: now + Duration::minutes(15), reserved_at: now,
    }, "SeatsReserved")]).await.unwrap();
    ip.project(&[ser(&InventoryEvent::SeatsReleased {
        reservation_id: res_id3, seats: seats3[0..3].to_vec(),
        reason: ReleaseReason::CustomerCancelled, released_at: now,
    }, "SeatsReleased")]).await.unwrap();
    let avail = iq.get_section_availability(ev_id3, "Floor").await.unwrap().unwrap();
    assert_eq!(avail.available_seats, 20);
    println!("  ✓ InventoryProjector: SeatsReleased (CustomerCancelled)");

    // 20. Idempotency - duplicate initialize
    reset(&pool).await;
    let ev_id4 = EventId::new();
    let seats4: Vec<SeatId> = (0..10).map(|_| SeatId::new()).collect();
    let init_ev = ser(&InventoryEvent::Initialized {
        event_id: ev_id4, section: "Test".into(), capacity: 10, seats: seats4.clone(), initialized_at: now,
    }, "InventoryInitialized");
    ip.project(std::slice::from_ref(&init_ev)).await.unwrap();
    ip.project(std::slice::from_ref(&init_ev)).await.unwrap(); // should be idempotent
    let avail = iq.get_section_availability(ev_id4, "Test").await.unwrap().unwrap();
    assert_eq!(avail.total_capacity, 10);
    println!("  ✓ InventoryProjector: Idempotent initialize");

    // ═══════════════════════════════════════════════════════════════════════
    // InventoryProjectionQueries - All Query Methods
    // ═══════════════════════════════════════════════════════════════════════

    // 21. get_section_availability not found
    reset(&pool).await;
    assert!(iq.get_section_availability(EventId::new(), "X").await.unwrap().is_none());
    println!("  ✓ InventoryQueries: get_section_availability not found");

    // 22. get_event_availability multiple sections
    reset(&pool).await;
    let multi_ev = EventId::new();
    for (section, cap) in [("VIP", 50u32), ("GA", 200), ("Balcony", 100)] {
        let s: Vec<SeatId> = (0..cap).map(|_| SeatId::new()).collect();
        ip.project(&[ser(&InventoryEvent::Initialized {
            event_id: multi_ev, section: section.into(), capacity: cap, seats: s, initialized_at: now,
        }, "InventoryInitialized")]).await.unwrap();
    }
    let sections = iq.get_event_availability(multi_ev).await.unwrap();
    assert_eq!(sections.len(), 3);
    let total_cap: u32 = sections.iter().map(|s| s.total_capacity).sum();
    assert_eq!(total_cap, 350);
    println!("  ✓ InventoryQueries: get_event_availability multiple sections");

    // 23. get_total_available
    let total = iq.get_total_available(multi_ev).await.unwrap();
    assert_eq!(total, 350);
    println!("  ✓ InventoryQueries: get_total_available");

    // 24. get_inventory_dto
    let inv_dto = iq.get_inventory_dto(multi_ev, "VIP").await.unwrap().unwrap();
    assert!(inv_dto.initialized);
    assert_eq!(inv_dto.capacity, 50);
    assert_eq!(inv_dto.available_count, 50);
    assert_eq!(inv_dto.available_seats.len(), 50);
    println!("  ✓ InventoryQueries: get_inventory_dto");

    // 25. get_inventory_dto not initialized
    assert!(iq.get_inventory_dto(EventId::new(), "X").await.unwrap().is_none());
    println!("  ✓ InventoryQueries: get_inventory_dto not found");

    // 26. get_reservation_dto with active reservation
    reset(&pool).await;
    let ev5 = EventId::new();
    let s5: Vec<SeatId> = (0..20).map(|_| SeatId::new()).collect();
    ip.project(&[ser(&InventoryEvent::Initialized {
        event_id: ev5, section: "S".into(), capacity: 20, seats: s5.clone(), initialized_at: now,
    }, "InventoryInitialized")]).await.unwrap();
    let res5 = ReservationId::new();
    let exp = now + Duration::minutes(15);
    ip.project(&[ser(&InventoryEvent::SeatsReserved {
        reservation_id: res5, event_id: ev5, section: "S".into(), seats: s5[0..4].to_vec(),
        expires_at: exp, reserved_at: now,
    }, "SeatsReserved")]).await.unwrap();
    let res_dto = iq.get_reservation_dto(res5).await.unwrap().unwrap();
    assert_eq!(res_dto.seats.len(), 4);
    assert_eq!(res_dto.expires_at, exp);
    println!("  ✓ InventoryQueries: get_reservation_dto active");

    // 27. get_reservation_dto not found
    assert!(iq.get_reservation_dto(ReservationId::new()).await.unwrap().is_none());
    println!("  ✓ InventoryQueries: get_reservation_dto not found");

    // ═══════════════════════════════════════════════════════════════════════
    // PaymentProjector - All Event Types
    // ═══════════════════════════════════════════════════════════════════════
    let pp = PaymentProjector::new(pool.clone());
    let pq = PaymentProjectionQueries::new(pool.clone());

    // 28. PaymentProcessed
    reset(&pool).await;
    let pay_id = PaymentId::new();
    let cust_id = CustomerId::new();
    let res_pay = ReservationId::new();
    pp.project(&[ser(&PaymentEvent::PaymentProcessed {
        payment_id: pay_id, reservation_id: res_pay, customer_id: cust_id,
        amount: Money::from_cents(15000),
        payment_method: PaymentMethod::CreditCard { last_four: "4242".into() },
        processed_at: now,
    }, "PaymentProcessed")]).await.expect("project");
    let dto = pq.get_payment(pay_id).await.unwrap().unwrap();
    assert_eq!(dto.amount.cents(), 15000);
    assert_eq!(dto.status, ticketing::next::PaymentDtoStatus::Processing);
    println!("  ✓ PaymentProjector: PaymentProcessed");

    // 29. PaymentSucceeded
    pp.project(&[ser(&PaymentEvent::PaymentSucceeded {
        payment_id: pay_id, transaction_id: "txn_abc123".into(), succeeded_at: now,
    }, "PaymentSucceeded")]).await.expect("project");
    let dto = pq.get_payment(pay_id).await.unwrap().unwrap();
    assert_eq!(dto.status, ticketing::next::PaymentDtoStatus::Succeeded);
    assert_eq!(dto.transaction_id.as_deref(), Some("txn_abc123"));
    println!("  ✓ PaymentProjector: PaymentSucceeded");

    // 30. PaymentFailed
    reset(&pool).await;
    let pay_id2 = PaymentId::new();
    pp.project(&[ser(&PaymentEvent::PaymentProcessed {
        payment_id: pay_id2, reservation_id: ReservationId::new(), customer_id: CustomerId::new(),
        amount: Money::from_cents(5000), payment_method: PaymentMethod::ApplePay, processed_at: now,
    }, "PaymentProcessed")]).await.unwrap();
    pp.project(&[ser(&PaymentEvent::PaymentFailed {
        payment_id: pay_id2, reason: "Card declined".into(), failed_at: now,
    }, "PaymentFailed")]).await.unwrap();
    let dto = pq.get_payment(pay_id2).await.unwrap().unwrap();
    assert_eq!(dto.status, ticketing::next::PaymentDtoStatus::Failed);
    assert_eq!(dto.failure_reason.as_deref(), Some("Card declined"));
    println!("  ✓ PaymentProjector: PaymentFailed");

    // 31. PaymentRefunded
    reset(&pool).await;
    let pay_id3 = PaymentId::new();
    pp.project(&[ser(&PaymentEvent::PaymentProcessed {
        payment_id: pay_id3, reservation_id: ReservationId::new(), customer_id: CustomerId::new(),
        amount: Money::from_cents(20000),
        payment_method: PaymentMethod::PayPal { email: "test@example.com".into() },
        processed_at: now,
    }, "PaymentProcessed")]).await.unwrap();
    pp.project(&[ser(&PaymentEvent::PaymentSucceeded {
        payment_id: pay_id3, transaction_id: "txn_xyz".into(), succeeded_at: now,
    }, "PaymentSucceeded")]).await.unwrap();
    pp.project(&[ser(&PaymentEvent::PaymentRefunded {
        payment_id: pay_id3, amount: Money::from_cents(20000), reason: "Event cancelled".into(), refunded_at: now,
    }, "PaymentRefunded")]).await.unwrap();
    let dto = pq.get_payment(pay_id3).await.unwrap().unwrap();
    assert_eq!(dto.status, ticketing::next::PaymentDtoStatus::Refunded);
    assert_eq!(dto.refund_amount.map(|m| m.cents()), Some(20000));
    assert_eq!(dto.refund_reason.as_deref(), Some("Event cancelled"));
    println!("  ✓ PaymentProjector: PaymentRefunded");

    // 32. Idempotency - duplicate processed
    reset(&pool).await;
    let pay_id4 = PaymentId::new();
    let proc_ev = ser(&PaymentEvent::PaymentProcessed {
        payment_id: pay_id4, reservation_id: ReservationId::new(), customer_id: CustomerId::new(),
        amount: Money::from_cents(1000), payment_method: PaymentMethod::ApplePay, processed_at: now,
    }, "PaymentProcessed");
    pp.project(std::slice::from_ref(&proc_ev)).await.unwrap();
    pp.project(std::slice::from_ref(&proc_ev)).await.unwrap(); // idempotent
    assert!(pq.get_payment(pay_id4).await.unwrap().is_some());
    println!("  ✓ PaymentProjector: Idempotent");

    // ═══════════════════════════════════════════════════════════════════════
    // PaymentProjectionQueries - All Query Methods
    // ═══════════════════════════════════════════════════════════════════════

    // 33. get_payment not found
    reset(&pool).await;
    assert!(pq.get_payment(PaymentId::new()).await.unwrap().is_none());
    println!("  ✓ PaymentQueries: get_payment not found");

    // 34. list_by_customer
    reset(&pool).await;
    let cust_list = CustomerId::new();
    let other_cust = CustomerId::new();
    for i in 0..4 {
        pp.project(&[ser(&PaymentEvent::PaymentProcessed {
            payment_id: PaymentId::new(), reservation_id: ReservationId::new(), customer_id: cust_list,
            amount: Money::from_cents(1000 * (i + 1)), payment_method: PaymentMethod::ApplePay, processed_at: now,
        }, "PaymentProcessed")]).await.unwrap();
    }
    pp.project(&[ser(&PaymentEvent::PaymentProcessed {
        payment_id: PaymentId::new(), reservation_id: ReservationId::new(), customer_id: other_cust,
        amount: Money::from_cents(9999), payment_method: PaymentMethod::ApplePay, processed_at: now,
    }, "PaymentProcessed")]).await.unwrap();
    let list = pq.list_by_customer(cust_list).await.unwrap();
    assert_eq!(list.len(), 4);
    for p in &list {
        assert_eq!(p.customer_id, cust_list);
    }
    println!("  ✓ PaymentQueries: list_by_customer");

    // 35. list_by_customer empty
    assert!(pq.list_by_customer(CustomerId::new()).await.unwrap().is_empty());
    println!("  ✓ PaymentQueries: list_by_customer empty");

    // ═══════════════════════════════════════════════════════════════════════
    // Edge Cases & Error Handling
    // ═══════════════════════════════════════════════════════════════════════

    // 36. Empty batch projection
    reset(&pool).await;
    ep.project(&[]).await.expect("empty batch events");
    ip.project(&[]).await.expect("empty batch inventory");
    pp.project(&[]).await.expect("empty batch payments");
    println!("  ✓ Edge: Empty batch projection");

    // 37. Special characters in event name
    reset(&pool).await;
    let special_id = EventId::new();
    ep.project(&[ser(&EventEvent::Created {
        event_id: special_id, name: "Taylor's \"Eras\" 北京 — 2025!".into(), owner_id: UserId::new(),
        venue: venue(), date: EventDate::new(now + Duration::days(30)),
        pricing_tiers: pricing(), created_at: now,
    }, "EventCreated")]).await.expect("project");
    let dto = eq.get_event(special_id).await.unwrap().unwrap();
    assert_eq!(dto.name, "Taylor's \"Eras\" 北京 — 2025!");
    println!("  ✓ Edge: Special characters in name");

    // 38. Large venue (many sections)
    reset(&pool).await;
    let large_ev = EventId::new();
    let large_venue = Venue {
        name: "Mega".into(),
        capacity: Capacity::new(5000),
        sections: (0..50).map(|i| VenueSection {
            name: format!("Section-{i}"), capacity: Capacity::new(100), seat_type: SeatType::GeneralAdmission,
        }).collect(),
    };
    ep.project(&[ser(&EventEvent::Created {
        event_id: large_ev, name: "Large".into(), owner_id: UserId::new(), venue: large_venue,
        date: EventDate::new(now + Duration::days(30)), pricing_tiers: pricing(), created_at: now,
    }, "EventCreated")]).await.expect("project");
    let dto = eq.get_event(large_ev).await.unwrap().unwrap();
    assert_eq!(dto.venue.sections.len(), 50);
    println!("  ✓ Edge: Large venue JSON");

    // 39. Multiple reservations same section
    reset(&pool).await;
    let multi_res_ev = EventId::new();
    let multi_seats: Vec<SeatId> = (0..100).map(|_| SeatId::new()).collect();
    ip.project(&[ser(&InventoryEvent::Initialized {
        event_id: multi_res_ev, section: "Main".into(), capacity: 100, seats: multi_seats.clone(), initialized_at: now,
    }, "InventoryInitialized")]).await.unwrap();
    for i in 0..5 {
        let chunk: Vec<SeatId> = multi_seats[(i*10)..(i*10+10)].to_vec();
        ip.project(&[ser(&InventoryEvent::SeatsReserved {
            reservation_id: ReservationId::new(), event_id: multi_res_ev, section: "Main".into(),
            seats: chunk, expires_at: now + Duration::minutes(15), reserved_at: now,
        }, "SeatsReserved")]).await.unwrap();
    }
    let avail = iq.get_section_availability(multi_res_ev, "Main").await.unwrap().unwrap();
    assert_eq!(avail.reserved_seats, 50);
    assert_eq!(avail.available_seats, 50);
    println!("  ✓ Edge: Multiple reservations same section");

    // 40. Payment method variants
    reset(&pool).await;
    for method in [
        PaymentMethod::CreditCard { last_four: "1234".into() },
        PaymentMethod::PayPal { email: "a@b.com".into() },
        PaymentMethod::ApplePay,
    ] {
        pp.project(&[ser(&PaymentEvent::PaymentProcessed {
            payment_id: PaymentId::new(), reservation_id: ReservationId::new(), customer_id: CustomerId::new(),
            amount: Money::from_cents(100), payment_method: method, processed_at: now,
        }, "PaymentProcessed")]).await.unwrap();
    }
    println!("  ✓ Edge: All payment method variants");

    println!("\n✅ All 40 projection tests passed!");
}
