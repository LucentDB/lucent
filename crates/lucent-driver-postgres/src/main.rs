use std::env;

#[tokio::main]
async fn main() {
    let socket_path = env::args()
        .nth(1)
        .expect("usage: lucent-driver-postgres <socket-path> <handshake-token>");
    let handshake_token = env::args()
        .nth(2)
        .expect("usage: lucent-driver-postgres <socket-path> <handshake-token>");

    let listener = lucent_worker_host::bind(&socket_path).expect("failed to bind worker socket");
    let connector = lucent_driver_postgres::PostgresConnector::default();

    lucent_worker_host::serve(listener, handshake_token, connector)
        .await
        .expect("worker loop exited with an error");
}
