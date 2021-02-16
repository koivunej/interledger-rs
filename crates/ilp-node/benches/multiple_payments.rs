//! This file has two huge startup fns which each setup either btp or http communication between
//! nodes. The only difference between the two are the account properties:
//!  - ilp_over_{btp,http}_url
//!  - ilp_over_{btp,http}_outgoing_token

use criterion::{criterion_group, criterion_main, Criterion};
use ilp_node::InterledgerNode;
use serde_json::{self, json};
use std::time::Duration;
use tokio::runtime::Runtime;
use tokio::sync::mpsc::channel;
use tokio::time::{delay_for, timeout};
use tungstenite::{client, handshake::client::Request};

mod redis_helpers;
mod test_helpers;

use redis_helpers::*;
use test_helpers::*;

/// The *_hundred_packets bench fns send 100 packets for which we expect to read 100 + 1 responses
/// from the websocket.
const BUFFER_SIZE: usize = 101;

/// There will be executions where routes seem to be propagated right away during init, but on most
/// runs they require this propagation delay. It will create additional noise to results.
/// FIXME: this is a workaround to difficult startup
const ROUTE_BROADCAST_INTERVAL: u64 = 500;

fn multiple_payments_btp(c: &mut Criterion) {
    let mut rt = Runtime::new().unwrap();

    let node_a_http = get_open_port(None);
    let node_a_settlement = get_open_port(None);
    let node_b_http = get_open_port(None);
    let node_b_settlement = get_open_port(None);
    let context = TestContext::new();

    let mut connection_info1 = context.get_client_connection_info();
    connection_info1.db = 1;
    let mut connection_info2 = context.get_client_connection_info();
    connection_info2.db = 2;

    // accounts to be created on node a
    let alice_on_a = json!({
        "username": "alice_on_a",
        "asset_code": "XYZ",
        "asset_scale": 9,
        "ilp_over_http_incoming_token" : "default account holder",
        "max_packet_amount": 100,
    });
    let b_on_a = json!({
        "username": "b_on_a",
        "asset_code": "XYZ",
        "asset_scale": 9,
        "ilp_over_btp_url": format!("ws://localhost:{}/accounts/{}/ilp/btp", node_b_http, "a_on_b"),
        "ilp_over_btp_outgoing_token" : "token",
        "routing_relation": "Parent",
    });

    // accounts to be created on node b
    let a_on_b = json!({
        "username": "a_on_b",
        "asset_code": "XYZ",
        "asset_scale": 9,
        "ilp_over_btp_incoming_token" : "token",
        "routing_relation": "Child",
    });
    let bob_on_b = json!({
        "username": "bob_on_b",
        "asset_code": "XYZ",
        "asset_scale": 9,
        "ilp_over_http_incoming_token" : "default account holder",
    });

    // node a config
    let node_a: InterledgerNode = serde_json::from_value(json!({
        "admin_auth_token": "admin",
        "database_url": connection_info_to_string(connection_info1),
        "http_bind_address": format!("127.0.0.1:{}", node_a_http),
        "settlement_api_bind_address": format!("127.0.0.1:{}", node_a_settlement),
        "secret_seed": random_secret(),
        "route_broadcast_interval": ROUTE_BROADCAST_INTERVAL,
        "exchange_rate": {
            "poll_interval": 60000
        },
    }))
    .expect("Error creating node_a.");

    // node b config
    let node_b: InterledgerNode = serde_json::from_value(json!({
        "ilp_address": "example.parent",
        "default_spsp_account": "bob_on_b",
        "admin_auth_token": "admin",
        "database_url": connection_info_to_string(connection_info2),
        "http_bind_address": format!("127.0.0.1:{}", node_b_http),
        "settlement_api_bind_address": format!("127.0.0.1:{}", node_b_settlement),
        "secret_seed": random_secret(),
        "route_broadcast_interval": ROUTE_BROADCAST_INTERVAL,
        "exchange_rate": {
            "poll_interval": 60000
        },
    }))
    .expect("Error creating node_b.");

    rt.block_on(
        // start node b and open its accounts
        async {
            node_b.serve(None).await.unwrap();
            create_account_on_node(node_b_http, a_on_b, "admin")
                .await
                .unwrap();
            create_account_on_node(node_b_http, bob_on_b, "admin")
                .await
                .unwrap();

            // start node a and open its accounts
            node_a.serve(None).await.unwrap();
            create_account_on_node(node_a_http, alice_on_a, "admin")
                .await
                .unwrap();
            create_account_on_node(node_a_http, b_on_a, "admin")
                .await
                .unwrap();
        },
    );

    let ws_request = Request::builder()
        .uri(format!("ws://localhost:{}/payments/incoming", node_b_http))
        .header("Authorization", "Bearer admin")
        .body(())
        .unwrap();

    let (sender, mut receiver) = channel(BUFFER_SIZE);
    let client = reqwest::Client::new();
    let req_low = client
        .post(&format!(
            "http://localhost:{}/accounts/{}/payments",
            node_a_http, "alice_on_a"
        ))
        .header(
            "Authorization",
            format!("Bearer {}", "default account holder"),
        )
        .json(&json!({
            "receiver": format!("http://localhost:{}/accounts/{}/spsp", node_b_http,"bob_on_b"),
            "source_amount": 100,
            "slippage": 0.025 // allow up to 2.5% slippage
        }));
    let req_high = client
        .post(&format!(
            "http://localhost:{}/accounts/{}/payments",
            node_a_http, "alice_on_a"
        ))
        .header(
            "Authorization",
            format!("Bearer {}", "default account holder"),
        )
        .json(&json!({
            "receiver": format!("http://localhost:{}/accounts/{}/spsp", node_b_http,"bob_on_b"),
            "source_amount": 10000,
            "slippage": 0.025 // allow up to 2.5% slippage
        }));

    let handle = std::thread::spawn(move || payment_notifications_shovel(ws_request, sender));

    rt.block_on(async {
        timeout(
            Duration::from_secs(5),
            node_readyness(&req_low, &mut receiver),
        )
        .await
        .expect("nodes did not become ready in time")
    });

    c.bench_function("process_payment_btp_single_packet", |b| {
        b.iter(|| bench_fn(&mut rt, &req_low, &mut receiver, 2))
    });

    c.bench_function("process_payment_btp_hundred_packets", |b| {
        b.iter(|| bench_fn(&mut rt, &req_high, &mut receiver, BUFFER_SIZE));
    });

    drop(rt);
    handle.join().unwrap().unwrap();
}

/// Shovel the payment notifications from the websocket to the channel until there's an error which
/// is expected to happen because the runtime powering the ilp-node was dropped.
fn payment_notifications_shovel(
    ws_request: tungstenite::http::Request<()>,
    mut sender: tokio::sync::mpsc::Sender<tungstenite::Message>,
) -> Result<(), tungstenite::Error> {
    let mut payments_ws = client::connect(ws_request)?.0;
    while let Ok(message) = payments_ws.read_message() {
        sender
            .try_send(message)
            .expect("failed to send message right away; is the BUFFER_SIZE correct?");
    }
    // dont send anything special; assume that the connection already failed
    payments_ws.close(None)
}

fn bench_fn(
    rt: &mut tokio::runtime::Runtime,
    req: &reqwest::RequestBuilder,
    receiver: &mut tokio::sync::mpsc::Receiver<tungstenite::Message>,
    expected_packets: usize,
) {
    rt.block_on(async {
        let response = req.try_clone().unwrap().send().await.unwrap();
        if !response.status().is_success() {
            // this error case happens only in the beginning of a benchmark, it is assumed it is
            // related to route propagation being faulty.
            let headers = response.headers().to_owned();
            match response.text().await {
                Ok(s) => panic!(
                    "Invalid response received: headers:\n\n{:?}\n\nbody:\n\n{}",
                    headers, s
                ),
                Err(e) => panic!(
                    "Invalid response received: headers:\n\n{:?}\n\nbody to text error: {}",
                    headers, e
                ),
            }
        }
        for _ in 0..expected_packets {
            receiver.recv().await.unwrap().into_text().unwrap();
        }
    });
}

fn multiple_payments_http(c: &mut Criterion) {
    let mut rt = Runtime::new().unwrap();
    let node_a_http = get_open_port(None);
    let node_a_settlement = get_open_port(None);
    let node_b_http = get_open_port(None);
    let node_b_settlement = get_open_port(None);
    let context = TestContext::new();

    let mut connection_info1 = context.get_client_connection_info();
    connection_info1.db = 1;
    let mut connection_info2 = context.get_client_connection_info();
    connection_info2.db = 2;

    // accounts to be created on node a
    let alice_on_a = json!({
        "username": "alice_on_a",
        "asset_code": "XYZ",
        "asset_scale": 9,
        "ilp_over_http_incoming_token" : "admin",
        "max_packet_amount": 100,
    });
    let b_on_a = json!({
        "username": "b_on_a",
        "asset_code": "XYZ",
        "asset_scale": 9,
        "ilp_over_http_url": format!("http://localhost:{}/accounts/{}/ilp", node_b_http, "a_on_b"),
        "ilp_over_http_incoming_token" : "admin",
        "ilp_over_http_outgoing_token" : "admin",
        "ilp_address": "example.node_b",
    });

    // accounts to be created on node b
    let a_on_b = json!({
        "username": "a_on_b",
        "ilp_over_http_url": format!("http://localhost:{}/accounts/{}/ilp", node_a_http, "b_on_a"),
        "asset_code": "XYZ",
        "asset_scale": 9,
        "ilp_over_http_incoming_token" : "admin",
        "ilp_over_http_outgoing_token" : "admin",
    });
    let bob_on_b = json!({
        "username": "bob_on_b",
        "asset_code": "XYZ",
        "asset_scale": 9,
        "ilp_over_http_incoming_token" : "admin",
    });

    let node_a: InterledgerNode = serde_json::from_value(json!({
        "ilp_address": "example.node_a",
        "secret_seed" : random_secret(),
        "admin_auth_token": "admin",
        "database_url": connection_info_to_string(connection_info1),
        "http_bind_address": format!("127.0.0.1:{}", node_a_http),
        "settlement_api_bind_address": format!("127.0.0.1:{}", node_a_settlement),
        "route_broadcast_interval": ROUTE_BROADCAST_INTERVAL,
    }))
    .expect("Error creating node_a.");

    let node_b: InterledgerNode = serde_json::from_value(json!({
        "ilp_address": "example.node_b",
        "secret_seed" : random_secret(),
        "admin_auth_token": "admin",
        "database_url": connection_info_to_string(connection_info2),
        "http_bind_address": format!("127.0.0.1:{}", node_b_http),
        "settlement_api_bind_address": format!("127.0.0.1:{}", node_b_settlement),
        "route_broadcast_interval": ROUTE_BROADCAST_INTERVAL,
    }))
    .expect("Error creating node_b.");
    rt.block_on(
        // start node b and open its accounts
        async {
            node_b.serve(None).await.unwrap();
            create_account_on_node(node_b_http, a_on_b, "admin")
                .await
                .unwrap();
            create_account_on_node(node_b_http, bob_on_b, "admin")
                .await
                .unwrap();

            // start node a and open its accounts
            node_a.serve(None).await.unwrap();
            create_account_on_node(node_a_http, alice_on_a, "admin")
                .await
                .unwrap();
            create_account_on_node(node_a_http, b_on_a, "admin")
                .await
                .unwrap();
        },
    );

    let ws_request = Request::builder()
        .uri(format!("ws://localhost:{}/payments/incoming", node_b_http))
        .header("Authorization", "Bearer admin")
        .body(())
        .unwrap();
    let (sender, mut receiver) = channel(BUFFER_SIZE);
    let client = reqwest::Client::new();
    let req_low = client
        .post(&format!(
            "http://localhost:{}/accounts/{}/payments",
            node_a_http, "alice_on_a"
        ))
        .header("Authorization", format!("Bearer {}", "admin"))
        .json(&json!({
            "receiver": format!("http://localhost:{}/accounts/{}/spsp", node_b_http,"bob_on_b"),
            "source_amount": 100,
            "slippage": 0.025 // allow up to 2.5% slippage
        }));
    let req_high = client
        .post(&format!(
            "http://localhost:{}/accounts/{}/payments",
            node_a_http, "alice_on_a"
        ))
        .header("Authorization", format!("Bearer {}", "admin"))
        .json(&json!({
            "receiver": format!("http://localhost:{}/accounts/{}/spsp", node_b_http,"bob_on_b"),
            "source_amount": 10000,
            "slippage": 0.025 // allow up to 2.5% slippage
        }));
    let handle = std::thread::spawn(move || payment_notifications_shovel(ws_request, sender));

    rt.block_on(async {
        timeout(
            Duration::from_secs(5),
            node_readyness(&req_low, &mut receiver),
        )
        .await
        .expect("nodes did not become ready in time")
    });

    c.bench_function("process_payment_http_single_packet", |b| {
        b.iter(|| bench_fn(&mut rt, &req_low, &mut receiver, 2))
    });
    c.bench_function("process_payment_http_hundred_packets", |b| {
        b.iter(|| bench_fn(&mut rt, &req_high, &mut receiver, BUFFER_SIZE));
    });

    drop(rt);
    handle.join().unwrap().unwrap();
}

// try until the routes have been propagated
async fn node_readyness(
    single_packet_req: &reqwest::RequestBuilder,
    receiver: &mut tokio::sync::mpsc::Receiver<tungstenite::Message>,
) {
    loop {
        let res = single_packet_req.try_clone().unwrap().send().await.unwrap();

        if !res.status().is_success() {
            tracing::warn!("still no success");
            delay_for(Duration::from_millis(ROUTE_BROADCAST_INTERVAL)).await;
            // this happens until the routes have been propagated
            continue;
        }

        let msg = receiver
            .recv()
            .await
            .expect("shoveller must still be alive")
            .into_text();

        tracing::info!("payment notification: {:?}", msg);
        // there will be the connectionclose notification

        let msg = receiver
            .recv()
            .await
            .expect("shoveller must still be alive")
            .into_text();

        tracing::info!("connectionclose notification: {:?}", msg);
        break;
    }
}

criterion_group!(benches, multiple_payments_http, multiple_payments_btp);
criterion_main!(benches);
