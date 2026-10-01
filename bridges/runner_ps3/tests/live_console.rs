//! The one test target that reaches a console. It builds only with the
//! `ps3-hardware` feature, and with it on, a missing `CELLGOV_PS3_HOST`
//! is a hard error, never a skip: the feature is the statement that a
//! console is on the network.

use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use runner_ps3::transport::Endpoint;

fn console() -> Endpoint {
    let host = std::env::var("CELLGOV_PS3_HOST")
        .expect("ps3-hardware is on, so CELLGOV_PS3_HOST must name the console");
    assert!(!host.is_empty(), "CELLGOV_PS3_HOST is set but empty");
    Endpoint::new(host)
}

#[test]
fn the_console_accepts_a_connection_on_webmans_http_port() {
    let endpoint = console();
    let address = (endpoint.host.as_str(), endpoint.http_port)
        .to_socket_addrs()
        .expect("the host resolves")
        .next()
        .expect("the host has an address");
    let timeout = Duration::from_millis(endpoint.io_timeout_ms);
    let stream = TcpStream::connect_timeout(&address, timeout).unwrap_or_else(|e| {
        panic!(
            "{}:{} did not accept a connection: {e}",
            endpoint.host, endpoint.http_port
        )
    });
    assert_eq!(
        stream.peer_addr().expect("peer address").port(),
        endpoint.http_port
    );
}
