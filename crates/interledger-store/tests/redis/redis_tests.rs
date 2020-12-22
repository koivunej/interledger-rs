mod accounts_test;
mod balances_test;
mod btp_test;
mod http_test;
mod rate_limiting_test;
mod rates_test;
mod routing_test;
mod settlement_test;

mod fixtures {

    use interledger_api::AccountDetails;
    use interledger_packet::Address;
    use interledger_service::Username;
    use once_cell::sync::Lazy;
    use secrecy::SecretString;
    use std::str::FromStr;

    // We are dylan starting a connection with all these accounts
    pub static ACCOUNT_DETAILS_0: Lazy<AccountDetails> = Lazy::new(|| AccountDetails {
        ilp_address: Some(Address::from_str("example.alice").unwrap()),
        username: Username::from_str("alice").unwrap(),
        asset_scale: 6,
        asset_code: "XYZ".to_string(),
        max_packet_amount: 1000,
        min_balance: Some(-1000),
        ilp_over_http_url: Some("http://example.com/accounts/dylan/ilp".to_string()),
        ilp_over_http_incoming_token: Some(SecretString::new("incoming_auth_token".to_string())),
        ilp_over_http_outgoing_token: Some(SecretString::new("outgoing_auth_token".to_string())),
        ilp_over_btp_url: Some("btp+ws://example.com/accounts/dylan/ilp/btp".to_string()),
        ilp_over_btp_incoming_token: Some(SecretString::new("btp_token".to_string())),
        ilp_over_btp_outgoing_token: Some(SecretString::new("btp_token".to_string())),
        settle_threshold: Some(0),
        settle_to: Some(-1000),
        routing_relation: Some("Parent".to_owned()),
        round_trip_time: None,
        amount_per_minute_limit: Some(1000),
        packets_per_minute_limit: Some(2),
        settlement_engine_url: Some("http://settlement.example".to_string()),
    });
    pub static ACCOUNT_DETAILS_1: Lazy<AccountDetails> = Lazy::new(|| AccountDetails {
        ilp_address: None,
        username: Username::from_str("bob").unwrap(),
        asset_scale: 9,
        asset_code: "ABC".to_string(),
        max_packet_amount: 1_000_000,
        min_balance: Some(0),
        ilp_over_http_url: Some("http://example.com/accounts/dylan/ilp".to_string()),
        // incoming token has is the account's username concatenated wiht the password
        ilp_over_http_incoming_token: Some(SecretString::new("incoming_auth_token".to_string())),
        ilp_over_http_outgoing_token: Some(SecretString::new("outgoing_auth_token".to_string())),
        ilp_over_btp_url: Some("btp+ws://example.com/accounts/dylan/ilp/btp".to_string()),
        ilp_over_btp_incoming_token: Some(SecretString::new("other_btp_token".to_string())),
        ilp_over_btp_outgoing_token: Some(SecretString::new("btp_token".to_string())),
        settle_threshold: Some(0),
        settle_to: Some(-1000),
        routing_relation: Some("Child".to_owned()),
        round_trip_time: None,
        amount_per_minute_limit: Some(1000),
        packets_per_minute_limit: Some(20),
        settlement_engine_url: None,
    });
    pub static ACCOUNT_DETAILS_2: Lazy<AccountDetails> = Lazy::new(|| AccountDetails {
        ilp_address: None,
        username: Username::from_str("charlie").unwrap(),
        asset_scale: 9,
        asset_code: "XRP".to_string(),
        max_packet_amount: 1000,
        min_balance: Some(0),
        ilp_over_http_url: None,
        ilp_over_http_incoming_token: None,
        ilp_over_http_outgoing_token: None,
        ilp_over_btp_url: None,
        ilp_over_btp_incoming_token: None,
        ilp_over_btp_outgoing_token: None,
        settle_threshold: Some(0),
        settle_to: None,
        routing_relation: None,
        round_trip_time: None,
        amount_per_minute_limit: None,
        packets_per_minute_limit: None,
        settlement_engine_url: None,
    });
}

use ilp_test_support::redis as redis_helpers;

mod store_helpers {
    use super::fixtures::*;
    use super::redis_helpers::*;

    use interledger_api::NodeStore;
    use interledger_packet::Address;
    use interledger_service::{Account as AccountTrait, AddressStore};
    use interledger_store::{
        account::Account,
        redis::{RedisStore, RedisStoreBuilder},
    };
    use std::str::FromStr;

    pub async fn test_store() -> Result<(RedisStore, TestContext, Vec<Account>), ()> {
        let context = TestContext::new();
        let store = RedisStoreBuilder::new(context.get_client_connection_info(), [0; 32])
            .node_ilp_address(Address::from_str("example.node").unwrap())
            .connect()
            .await
            .unwrap();
        let mut accs = Vec::new();
        let acc = store
            .insert_account(ACCOUNT_DETAILS_0.clone())
            .await
            .unwrap();
        accs.push(acc.clone());
        // alice is a Parent, so the store's ilp address is updated to
        // the value that would be received by the ILDCP request. here,
        // we just assume alice appended some data to her address
        store
            .set_ilp_address(acc.ilp_address().with_suffix(b"user1").unwrap())
            .await
            .unwrap();

        let acc = store
            .insert_account(ACCOUNT_DETAILS_1.clone())
            .await
            .unwrap();
        accs.push(acc);
        Ok((store, context, accs))
    }
}
