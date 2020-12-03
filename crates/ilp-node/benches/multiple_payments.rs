use criterion::{criterion_group, criterion_main, Criterion};
use ilp_node::InterledgerNode;
use serde_json::{self, json};
use tokio::runtime::Runtime;
use tokio::sync::mpsc::channel;
use tungstenite::{client, handshake::client::Request};

mod redis_helpers;
mod test_helpers;

use redis_helpers::*;
use test_helpers::*;

fn criterion_benchmark(c: &mut Criterion) {
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
    let caleb_on_b = json!({
        "username": "caleb_on_b",
        "asset_code": "XYZ",
        "asset_scale": 9,
        "ilp_over_http_incoming_token" : "default account holder",
    });
    let dave_on_b = json!({
        "username": "dave_on_b",
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
        "route_broadcast_interval": 200,
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
        "route_broadcast_interval": Some(200),
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
            create_account_on_node(node_b_http, caleb_on_b, "admin")
                .await
                .unwrap();
            create_account_on_node(node_b_http, dave_on_b, "admin")
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
    let (mut sender, mut receiver) = channel(100);
    let client = reqwest::Client::new();
    let req = client
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
            "source_amount": 1000,
            "slippage": 0.025 // allow up to 2.5% slippage
        }));
    let handle = std::thread::spawn(move || {
        let mut payments_ws = client::connect(ws_request).unwrap().0;
        while let Ok(message) = payments_ws.read_message(){
            sender.try_send(message).unwrap();
        }
        payments_ws.close(None).unwrap();
    });

    c.bench_function("send money", |b| {
        b.iter(|| {
            rt.block_on(async {
                req.try_clone().unwrap().send().await.unwrap();
                receiver.recv().await.unwrap().into_text().unwrap();
            });
        })
    });
    drop(rt);
    handle.join().unwrap();
}
criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
