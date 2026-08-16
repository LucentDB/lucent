//! End-to-end SSH tunnel: local port → sshd container → Postgres container.
//! Requires Docker; runs under `--features integration-tests`.
#![cfg(feature = "integration-tests")]

use std::path::Path;
use std::time::Duration;
use testcontainers::core::IntoContainerPort;
use testcontainers::CopyTargetOptions;
use testcontainers::{runners::AsyncRunner, ContainerAsync, GenericImage, ImageExt};

const SSH_IMAGE: &str = "linuxserver/openssh-server";
const POSTGRES_IMAGE: &str = "16-alpine";

/// The sshd container listens on port 2222 internally (linuxserver default);
/// the host port is a random exposed mapping.
async fn start_sshd() -> (ContainerAsync<GenericImage>, u16) {
    let sshd = GenericImage::new(SSH_IMAGE, "latest")
        .with_exposed_port(2222.tcp()) // container's sshd listens on 2222
        // The image defaults to `AllowTcpForwarding no`, which rejects the
        // direct-tcpip channel the tunnel needs — override /config/sshd/sshd_config.
        .with_copy_to(
            CopyTargetOptions::new("/config/sshd/sshd_config").with_mode(0o600),
            Path::new(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/sshd_config"
            )),
        )
        .with_env_var("USER_NAME", "tester")
        .with_env_var("USER_PASSWORD", "s3cret")
        .with_env_var("PASSWORD_ACCESS", "true") // docs.linuxserver.io: allow user/password ssh
        .with_env_var("PUBLIC_KEY", include_str!("fixtures/id_ed25519.pub").trim());
    let container = sshd.start().await.expect("sshd container starts");
    let port = container.get_host_port_ipv4(2222).await.unwrap();
    (container, port)
}

async fn start_postgres() -> ContainerAsync<GenericImage> {
    let pg = GenericImage::new("postgres", POSTGRES_IMAGE)
        .with_env_var("POSTGRES_PASSWORD", "pw")
        .with_env_var("POSTGRES_DB", "lucent");
    pg.start().await.expect("postgres container starts")
}

fn password_config(port: u16) -> lucent_lib::ssh::SshConfig {
    lucent_lib::ssh::SshConfig {
        id: "test-tunnel".into(),
        label: "Test tunnel".into(),
        host: "127.0.0.1".into(),
        port,
        user: "tester".into(),
        auth_method: lucent_lib::ssh::SshAuthMethod::Password,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn ssh_tunnel_roundtrips_postgres_queries_and_cancels_cleanly() {
    let (sshd, ssh_port) = start_sshd().await;
    let pg = start_postgres().await;
    // The docker default bridge has NO DNS between containers, so the tunnel
    // target is the postgres container's bridge IP — the sshd container
    // reaches it by IP exactly like a remote host.
    let pg_ip = pg.get_bridge_ip_address().await.unwrap();

    // sshd takes a couple of seconds to become ready — retry with backoff,
    // mirroring the repo's connect_test pattern.
    let mut tunnel = None;
    for attempt in 0..10 {
        match lucent_lib::ssh::SshTunnel::connect(
            &password_config(ssh_port),
            "s3cret",
            &pg_ip.to_string(),
            5432,
        )
        .await
        {
            Ok(t) => {
                tunnel = Some(t);
                break;
            }
            Err(e) if attempt < 9 => {
                eprintln!("tunnel attempt {attempt} failed ({e}); retrying");
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            Err(e) => panic!("tunnel connect failed after retries: {e}"),
        }
    }
    let mut tunnel = tunnel.expect("tunnel connects");

    // Connect through the tunnel exactly like a real DB connection.
    // Postgres takes ~2-3s after start() to accept connections (repo's
    // established pattern — connect_test retries with backoff).
    let mut client: Option<tokio_postgres::Client> = None;
    for attempt in 0..10 {
        let conn = tokio_postgres::connect(
            &format!(
                "host=127.0.0.1 port={} user=postgres password=pw dbname=lucent",
                tunnel.local_port
            ),
            tokio_postgres::NoTls,
        )
        .await;
        match conn {
            Ok((c, conn)) => {
                let _ = tokio::spawn(conn);
                client = Some(c);
                break;
            }
            Err(e) if attempt < 9 => {
                eprintln!("postgres attempt {attempt} failed ({e}); retrying");
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            Err(e) => panic!("postgres through tunnel failed after retries: {e}"),
        }
    }
    let client = client.expect("postgres connects through the tunnel");

    let row = client
        .query_one("SELECT 40 + 2", &[])
        .await
        .expect("full-duplex query round-trip");
    assert_eq!(row.get::<_, i32>(0), 42);

    // Cancellation: start a slow query, cancel it, expect it to abort fast.
    let stmt = client.prepare("SELECT pg_sleep(30)").await.unwrap();
    let cancel_token = client.cancel_token();
    let slow = tokio::spawn(async move { client.query(&stmt, &[]).await });
    tokio::time::sleep(Duration::from_millis(200)).await;
    cancel_token
        .cancel_query(tokio_postgres::NoTls)
        .await
        .expect("cancel request reaches the server");
    let started = std::time::Instant::now();
    assert!(slow.await.unwrap().is_err(), "slow query must be cancelled");
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "cancel must propagate through the tunnel quickly"
    );

    // Shutdown terminates fast.
    let t0 = std::time::Instant::now();
    tunnel.disconnect().await;
    assert!(t0.elapsed() < Duration::from_millis(100), "shutdown <100ms");

    let _ = (sshd, pg); // containers dropped at scope end
}

#[tokio::test(flavor = "multi_thread")]
async fn ssh_tunnel_rejects_bad_credentials() {
    let sshd = start_sshd().await;
    let err = lucent_lib::ssh::SshTunnel::connect(
        &password_config(sshd.1),
        "wrong-password",
        "postgres",
        5432,
    )
    .await;
    assert!(err.is_err(), "bad password must fail auth");
    let _ = sshd;
}
