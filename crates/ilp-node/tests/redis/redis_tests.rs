#![type_length_limit = "10000000"]
mod btp;
mod exchange_rates;
mod payments_incoming;
mod three_nodes;

// Only run prometheus tests if the monitoring feature is turned on
#[cfg(feature = "monitoring")]
mod prometheus;

use test_support::internal as test_helpers;
use test_support::redis as redis_helpers;
