use super::*;
use soroban_sdk::{testutils::Address as _, Address, Env};

struct Setup {
    env: Env,
    client: TicketRedeemerClient<'static>,
    admin: Address,
    token: Address,
}

fn setup() -> Setup {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(TicketRedeemer, ());
    let client = TicketRedeemerClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let token = Address::generate(&env);

    let client: TicketRedeemerClient<'static> = unsafe { core::mem::transmute(client) };

    Setup {
        env,
        client,
        admin,
        token,
    }
}

#[test]
fn test_init_and_snapshot() {
    let s = setup();
    s.client.init(&s.admin, &s.token, &100, &10);

    let snapshot = s.client.get_queue_snapshot();
    assert!(snapshot.config_initialized);
    assert_eq!(snapshot.total_entries, 0);
    assert_eq!(snapshot.pending_count, 0);
    assert_eq!(snapshot.redeemed_count, 0);
    assert_eq!(snapshot.expired_count, 0);
    assert!(!snapshot.is_paused);
    assert!(!snapshot.has_scan_window);
}

#[test]
fn test_submit_and_query() {
    let s = setup();
    s.client.init(&s.admin, &s.token, &100, &10);

    let user = Address::generate(&s.env);
    s.client.submit_ticket(&user, &0);
    s.client.submit_ticket(&user, &0);

    let snapshot = s.client.get_queue_snapshot();
    assert_eq!(snapshot.total_entries, 2);
    assert_eq!(snapshot.pending_count, 2);

    let entry = s.client.get_queue_entry(&0);
    assert!(entry.exists);
    assert_eq!(entry.ticket_id, Some(0));
    assert_eq!(entry.status, TicketStatus::Pending);
}

#[test]
fn test_redeem_and_snapshot() {
    let s = setup();
    s.client.init(&s.admin, &s.token, &100, &10);

    let user = Address::generate(&s.env);
    s.client.submit_ticket(&user, &0);
    s.client.redeem_ticket(&user, &0);

    let snapshot = s.client.get_queue_snapshot();
    assert_eq!(snapshot.pending_count, 0);
    assert_eq!(snapshot.redeemed_count, 1);

    let entry = s.client.get_queue_entry(&0);
    assert_eq!(entry.status, TicketStatus::Redeemed);
    assert!(entry.redeemed_at.is_some());
}

#[test]
fn test_scan_window_lifecycle() {
    let s = setup();
    s.client.init(&s.admin, &s.token, &100, &10);

    s.client.start_scan_window(&s.admin);
    let window = s.client.get_scan_window();
    assert!(window.is_some());
    assert!(window.unwrap().is_active);

    s.client.close_scan_window(&s.admin);
    let window = s.client.get_scan_window();
    assert!(window.is_some());
    assert!(!window.unwrap().is_active);
}

#[test]
fn test_uninitialized_reads() {
    let env = Env::default();
    let contract_id = env.register(TicketRedeemer, ());
    let client = TicketRedeemerClient::new(&env, &contract_id);

    let snapshot = client.get_queue_snapshot();
    assert!(!snapshot.config_initialized);
    assert!(snapshot.is_paused);
    assert_eq!(snapshot.total_entries, 0);

    let entry = client.get_queue_entry(&999);
    assert!(!entry.exists);

    assert!(client.get_scan_window().is_none());
    assert!(client.is_paused());
}

#[test]
fn test_paused_state_in_snapshot() {
    let s = setup();
    s.client.init(&s.admin, &s.token, &100, &10);

    s.client.set_pause(&true);
    let snapshot = s.client.get_queue_snapshot();
    assert!(snapshot.is_paused);
    assert!(s.client.is_paused());
}

#[test]
fn test_missing_entry() {
    let s = setup();
    s.client.init(&s.admin, &s.token, &100, &10);

    let entry = s.client.get_queue_entry(&999);
    assert!(!entry.exists);
    assert!(entry.ticket_id.is_none());
    assert!(entry.owner.is_none());
    assert_eq!(entry.status, TicketStatus::Pending);
}

#[test]
fn test_redemption_markers_prevent_replay() {
    let s = setup();
    s.client.init(&s.admin, &s.token, &100, &10);
    let user = Address::generate(&s.env);
    let imposter = Address::generate(&s.env);

    let ticket_id = 0u64;
    s.client.submit_ticket(&user, &ticket_id);

    // Imposter cannot redeem
    let err = s.client.try_redeem_ticket(&imposter, &ticket_id);
    assert_eq!(err, Err(Ok(Error::NotAuthorized)));

    // User redeems successfully
    assert!(s.client.try_redeem_ticket(&user, &ticket_id).is_ok());

    // Replay / duplicate redemption rejected
    let replay_err = s.client.try_redeem_ticket(&user, &ticket_id);
    assert_eq!(replay_err, Err(Ok(Error::InvalidTicket)));

    // Cancelled ticket cannot be redeemed
    let ticket_id2 = 1u64;
    s.client.submit_ticket(&user, &ticket_id2);
    s.client.cancel_ticket(&s.admin, &ticket_id2);
    let cancel_err = s.client.try_redeem_ticket(&user, &ticket_id2);
    assert_eq!(cancel_err, Err(Ok(Error::InvalidTicket)));
}

#[test]
fn test_queue_history_pagination_bounded_and_mixed() {
    let s = setup();
    s.client.init(&s.admin, &s.token, &100, &10);
    let user = Address::generate(&s.env);

    // Empty pagination
    let empty_page = s.client.get_queue_entries(&0, &10);
    assert_eq!(empty_page.len(), 0);

    // Submit 4 tickets
    for i in 0..4 {
        s.client.submit_ticket(&user, &i);
    }

    // Redeem ticket 0, cancel ticket 1, leave 2 and 3 pending
    s.client.redeem_ticket(&user, &0);
    s.client.cancel_ticket(&s.admin, &1);

    // Page 1: offset 0, limit 2
    let page1 = s.client.get_queue_entries(&0, &2);
    assert_eq!(page1.len(), 2);
    assert_eq!(page1.get(0).unwrap().status, TicketStatus::Redeemed);
    assert_eq!(page1.get(1).unwrap().status, TicketStatus::Cancelled);

    // Page 2: offset 2, limit 2
    let page2 = s.client.get_queue_entries(&2, &2);
    assert_eq!(page2.len(), 2);
    assert_eq!(page2.get(0).unwrap().status, TicketStatus::Pending);
    assert_eq!(page2.get(1).unwrap().status, TicketStatus::Pending);

    // Stale/out-of-bounds offset
    let oob_page = s.client.get_queue_entries(&10, &5);
    assert_eq!(oob_page.len(), 0);

    // Summary reflects accurate breakdown
    let summary = s.client.get_redemption_summary();
    assert_eq!(summary.total_redeemed, 1);
    assert_eq!(summary.total_cancelled, 1);
    assert_eq!(summary.total_pending, 2);
}
