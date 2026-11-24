#![cfg(test)]

use super::*;
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Env,
};
use types::StreamStatus;

fn setup(env: &Env) -> (TokenStreamingContractClient<'static>, Address, Address) {
    let contract_id = env.register(TokenStreamingContract, ());
    let client = TokenStreamingContractClient::new(env, &contract_id);
    let sender = Address::generate(env);
    let recipient = Address::generate(env);
    (client, sender, recipient)
}

#[test]
fn test_partial_withdrawal_halfway_through_stream() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, sender, recipient) = setup(&env);

    env.ledger().with_mut(|l| l.timestamp = 1000);
    let stream_id = client.create_stream(&sender, &recipient, &1000, &1000, &2000);

    // Halfway through (t=1500): 500 of 1000 streamed.
    env.ledger().with_mut(|l| l.timestamp = 1500);
    let available = client.get_available_balance(&stream_id);
    assert_eq!(available, 500);

    let withdrawn = client.withdraw_from_stream(&stream_id, &recipient, &300);
    assert_eq!(withdrawn, 300);

    let summary = client.get_stream(&stream_id);
    assert_eq!(summary.withdrawn, 300);
    assert_eq!(client.get_available_balance(&stream_id), 200);
}

#[test]
fn test_full_withdrawal_after_stream_completion() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, sender, recipient) = setup(&env);

    env.ledger().with_mut(|l| l.timestamp = 1000);
    let stream_id = client.create_stream(&sender, &recipient, &1000, &1000, &2000);

    env.ledger().with_mut(|l| l.timestamp = 2000);
    let withdrawn = client.withdraw_from_stream(&stream_id, &recipient, &1000);
    assert_eq!(withdrawn, 1000);

    let summary = client.get_stream(&stream_id);
    assert_eq!(summary.status, StreamStatus::Completed);
    assert_eq!(client.get_available_balance(&stream_id), 0);
}

#[test]
fn test_withdrawal_past_stop_ts_caps_at_deposit() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, sender, recipient) = setup(&env);

    env.ledger().with_mut(|l| l.timestamp = 1000);
    let stream_id = client.create_stream(&sender, &recipient, &1000, &1000, &2000);

    // Well past stop_ts — streamed amount must not exceed the deposit.
    env.ledger().with_mut(|l| l.timestamp = 5000);
    assert_eq!(client.get_available_balance(&stream_id), 1000);
}

#[test]
#[should_panic(expected = "amount exceeds available streamed balance")]
fn test_cannot_overdraw_stream() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, sender, recipient) = setup(&env);

    env.ledger().with_mut(|l| l.timestamp = 1000);
    let stream_id = client.create_stream(&sender, &recipient, &1000, &1000, &2000);

    env.ledger().with_mut(|l| l.timestamp = 1500);
    // Only 500 available at the halfway mark.
    client.withdraw_from_stream(&stream_id, &recipient, &600);
}

#[test]
fn test_cancellation_by_sender_splits_funds_by_elapsed_seconds() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, sender, recipient) = setup(&env);

    env.ledger().with_mut(|l| l.timestamp = 1000);
    let stream_id = client.create_stream(&sender, &recipient, &1000, &1000, &2000);

    // 30% elapsed (t=1300).
    env.ledger().with_mut(|l| l.timestamp = 1300);
    let (sender_refund, recipient_payout) = client.cancel_stream(&stream_id, &sender);

    assert_eq!(recipient_payout, 300);
    assert_eq!(sender_refund, 700);
    assert_eq!(sender_refund + recipient_payout, 1000);

    let summary = client.get_stream(&stream_id);
    assert_eq!(summary.status, StreamStatus::Cancelled);
}

#[test]
fn test_cancellation_by_recipient_after_partial_withdrawal() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, sender, recipient) = setup(&env);

    env.ledger().with_mut(|l| l.timestamp = 1000);
    let stream_id = client.create_stream(&sender, &recipient, &1000, &1000, &2000);

    env.ledger().with_mut(|l| l.timestamp = 1500);
    client.withdraw_from_stream(&stream_id, &recipient, &400);

    // Cancel at the same instant — 500 total streamed, 400 already
    // withdrawn, so only 100 more is owed to the recipient on cancellation.
    let (sender_refund, recipient_payout) = client.cancel_stream(&stream_id, &recipient);
    assert_eq!(recipient_payout, 100);
    assert_eq!(sender_refund, 500);
}

#[test]
#[should_panic(expected = "only the sender or recipient")]
fn test_third_party_cannot_cancel_stream() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, sender, recipient) = setup(&env);
    let stranger = Address::generate(&env);

    env.ledger().with_mut(|l| l.timestamp = 1000);
    let stream_id = client.create_stream(&sender, &recipient, &1000, &1000, &2000);

    client.cancel_stream(&stream_id, &stranger);
}

#[test]
#[should_panic(expected = "stream is not active")]
fn test_cannot_withdraw_from_cancelled_stream() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, sender, recipient) = setup(&env);

    env.ledger().with_mut(|l| l.timestamp = 1000);
    let stream_id = client.create_stream(&sender, &recipient, &1000, &1000, &2000);
    client.cancel_stream(&stream_id, &sender);

    client.withdraw_from_stream(&stream_id, &recipient, &1);
}

#[test]
fn test_cancellation_rounding_at_start_and_end_boundaries() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, sender, recipient) = setup(&env);

    // 1. Indivisible deposit across duration: deposit = 100, duration = 30 seconds (1000..1030)
    env.ledger().with_mut(|l| l.timestamp = 900);
    let s1 = client.create_stream(&sender, &recipient, &100, &1000, &1030);

    // Cancellation before start (t=950): sender gets full refund, recipient gets 0
    env.ledger().with_mut(|l| l.timestamp = 950);
    let (refund1, payout1) = client.cancel_stream(&s1, &sender);
    assert_eq!(refund1, 100);
    assert_eq!(payout1, 0);

    // 2. Cancellation exactly at start boundary (t=1000)
    let s2 = client.create_stream(&sender, &recipient, &100, &1000, &1030);
    env.ledger().with_mut(|l| l.timestamp = 1000);
    let (refund2, payout2) = client.cancel_stream(&s2, &recipient);
    assert_eq!(refund2, 100);
    assert_eq!(payout2, 0);

    // 3. Midpoint with integer division remainder: at t=1010 (10/30 elapsed -> streamed = 100 * 10 / 30 = 33)
    let s3 = client.create_stream(&sender, &recipient, &100, &1000, &1030);
    env.ledger().with_mut(|l| l.timestamp = 1010);
    let (refund3, payout3) = client.cancel_stream(&s3, &sender);
    assert_eq!(payout3, 33);
    assert_eq!(refund3, 67);
    assert_eq!(payout3 + refund3, 100);

    // 4. Cancellation at/after stop boundary (t=1035 >= 1030)
    let s4 = client.create_stream(&sender, &recipient, &100, &1000, &1030);
    env.ledger().with_mut(|l| l.timestamp = 1035);
    let (refund4, payout4) = client.cancel_stream(&s4, &recipient);
    assert_eq!(payout4, 100);
    assert_eq!(refund4, 0);
}

#[test]
fn test_vesting_arithmetic_overflow_safe() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, sender, recipient) = setup(&env);

    // Huge deposit that fits in u128 but deposit * elapsed exceeds u128
    let huge_deposit: u128 = u128::MAX / 2;
    let stream_id = client.create_stream(&sender, &recipient, &huge_deposit, &1000, &3000);

    // Midpoint timestamp 2000: mathematically vested amount is huge_deposit / 2
    env.ledger().set_timestamp(2000);
    let vested = client.get_available_balance(&stream_id);
    assert_eq!(vested, huge_deposit / 2);
}
