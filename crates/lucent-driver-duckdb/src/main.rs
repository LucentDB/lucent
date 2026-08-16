//! DuckDB worker binary.
//!
//! Spawned by the app's supervisor with a socket path and a handshake token,
//! exactly like `lucent-driver-postgres`. One process multiplexes every DuckDB
//! connection as a set of blocking tasks.

use lucent_driver_duckdb::connector::DuckDbConnector;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let mut args = std::env::args().skip(1);
    let Some(socket_path) = args.next() else {
        eprintln!("usage: lucent-driver-duckdb <socket-path> <handshake-token>");
        std::process::exit(2);
    };
    let Some(token) = args.next() else {
        eprintln!("usage: lucent-driver-duckdb <socket-path> <handshake-token>");
        std::process::exit(2);
    };

    let listener = lucent_worker_host::bind(&socket_path)?;
    lucent_worker_host::serve(listener, token, DuckDbConnector::default()).await
}
