#![type_length_limit = "10000000"]
mod btp;
mod exchange_rates;
mod payments_incoming;
mod three_nodes;

// Only run prometheus tests if the monitoring feature is turned on
#[cfg(feature = "monitoring")]
mod prometheus;

use ilp_test_support as test_helpers;
use redis_support as redis_helpers;
