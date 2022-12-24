use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::fs;
use std::sync::Arc;
use std::sync::Mutex;

use futures_util::sink::SinkExt; // for send
use futures_util::stream::StreamExt; // for split
use futures_util::stream::TryStreamExt; // for try_for_each

type PeerMap = Arc<Mutex<HashMap<std::net::SocketAddr, futures_channel::mpsc::UnboundedSender<String>>>>;

async fn handle_connection(peer_map: PeerMap, addr: std::net::SocketAddr, raw_stream: tokio::net::TcpStream, acceptor: tokio_native_tls::TlsAcceptor) {
  println!("{}: Connecting...", addr);

  let tls_stream: Result<tokio_native_tls::TlsStream<tokio::net::TcpStream>, native_tls::Error> = acceptor.accept(raw_stream).await;
  if let Err(error) = tls_stream {
    println!("{}: TLS handshake failed: {}", addr, error);
    return;
  }
  let tls_stream = tls_stream.unwrap();

  let ws_stream: Result<tokio_tungstenite::WebSocketStream<tokio_native_tls::TlsStream<tokio::net::TcpStream>>, tungstenite::Error> = tokio_tungstenite::accept_async(tls_stream).await;
  if let Err(error) = ws_stream {
    println!("{}: WebSocket handshake failed: {}", addr, error);
    return;
  }
  let (mut websocket_tx, websocket_rx) = ws_stream.unwrap().split();

  let (internal_tx, mut internal_rx) = futures_channel::mpsc::unbounded::<String>();
  peer_map.lock().unwrap().insert(addr, internal_tx);
  println!("{}: Connection established.", addr);

  let incoming_websocket_messages = websocket_rx.try_for_each(|message| {
    match message {
      tungstenite::protocol::Message::Text(payload) => {
        println!("{}: \"{}\"", addr, payload);
        // Forward the message to every other peer.
        let peers = peer_map.lock().unwrap();
        let peer_txs = peers.iter().filter(|(peer_addr, _peer_tx)| peer_addr != &&addr).map(|(_peer_addr, peer_tx)| peer_tx);
        for peer_tx in peer_txs {
          peer_tx.unbounded_send(payload.clone()).unwrap_or_default();
        }
      },
      tungstenite::protocol::Message::Close(payload) => {
        let reason: String;
        match payload {
          None => { reason = String::from("no reason provided"); },
          Some(data) => {
            if data.reason == "" {
              reason = match data.code {
                tungstenite::protocol::frame::coding::CloseCode::Normal => String::from("1000, client closed conection"),
                tungstenite::protocol::frame::coding::CloseCode::Away => String::from("1001, client was terminated"),
                tungstenite::protocol::frame::coding::CloseCode::Protocol => String::from("1002, server violated the protocol"),
                tungstenite::protocol::frame::coding::CloseCode::Unsupported => String::from("1003, server used sent an unexpected data type"),
                tungstenite::protocol::frame::coding::CloseCode::Status => String::from("no status received"),
                tungstenite::protocol::frame::coding::CloseCode::Abnormal => String::from("no close frame received"),
                tungstenite::protocol::frame::coding::CloseCode::Invalid => String::from("1007, server sent an erroneous payload"),
                tungstenite::protocol::frame::coding::CloseCode::Policy => String::from("1008, server violated a client policy"),
                tungstenite::protocol::frame::coding::CloseCode::Size => String::from("1009, payload too large for client"),
                tungstenite::protocol::frame::coding::CloseCode::Extension => String::from("1010, expected extension unspecified by server"),
                tungstenite::protocol::frame::coding::CloseCode::Error => String::from("1011, client suffered an internal error"),
                // Restart is for servers to report that they are restarting.
                // Again is for servers to report that they are overloaded.
                // 1014 is for servers who are acting as a gateway or proxy and received an invalid response from the upstream server to report that failure.
                // Tls is the internal code for "TLS handshake error", which is not relevant here since we do TLS separately.
                _ => format!("{}, no reason provided", data.code),
              };
            } else {
              reason = format!("{}: \"{}\"", data.code, data.reason);
            }
          }
        }
        println!("{}: Disconnecting... ({})", addr, reason);
      },
      _ => { } // binary, ping, and pong frames are ignored
    }
    futures_util::future::ok(())
  });

  let incoming_peer_messages = async move {
    while let Some(message) = internal_rx.next().await {
      websocket_tx.send(tungstenite::protocol::Message::Text(message.clone())).await.unwrap_or_default();
    };
  };

  futures_util::pin_mut!(incoming_websocket_messages, incoming_peer_messages);
  futures_util::future::select(incoming_websocket_messages, incoming_peer_messages).await;

  println!("{}: Disconnected.", &addr);
  peer_map.lock().unwrap().remove(&addr);
}

#[derive(Clone)]
struct Configuration {
  pfx_filename: String,
  server_address: String,
  server_port: u16,
  update_interval: core::time::Duration,
}

async fn read_configuration() -> Configuration {
  let configfile = fs::read_to_string("configuration").unwrap();
  let lines: Vec<&str> = configfile.split('\n').collect();
  Configuration{
    pfx_filename: String::from(lines[0]),
    server_address: String::from(lines[1]),
    server_port: lines[2].parse().expect("Port (line 3) was not an integer in the range 0..65535."),
    update_interval: core::time::Duration::from_secs(lines[3].parse().expect("Update interval (line 4) was not an integer (number of seconds).")),
  }
}

async fn prepare_acceptor(configuration: &Configuration) -> tokio_native_tls::TlsAcceptor {
  // Read the private key and certificate.
  println!("Refreshing certificates...");
  let mut file = File::open(&configuration.pfx_filename).unwrap();
  let mut pkcs12 = vec![];
  file.read_to_end(&mut pkcs12).unwrap();
  let pkcs12 = native_tls::Identity::from_pkcs12(&pkcs12, "").unwrap();
  tokio_native_tls::TlsAcceptor::from(native_tls::TlsAcceptor::builder(pkcs12).build().unwrap())
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
  let configuration = Arc::new(read_configuration().await);
  let state = PeerMap::new(Mutex::new(HashMap::new()));
  let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", configuration.server_port)).await.unwrap();

  let acceptor: Arc<Mutex<tokio_native_tls::TlsAcceptor>> = Arc::new(Mutex::new(prepare_acceptor(&configuration).await));
  tokio::spawn({
    let configuration: Arc<Configuration> = configuration.clone();
    let acceptor: Arc<Mutex<tokio_native_tls::TlsAcceptor>> = acceptor.clone();
    async move {
      let duration = configuration.update_interval;
      let mut interval = tokio::time::interval_at(tokio::time::Instant::now() + duration, duration);
      loop {
        interval.tick().await;
        let new_acceptor = prepare_acceptor(&configuration).await;
        *(acceptor.lock().unwrap()) = new_acceptor;
      }
    }
  });

  tokio::spawn({
    println!("Listening on: {}:{}", configuration.server_address, configuration.server_port);
    let acceptor: Arc<Mutex<tokio_native_tls::TlsAcceptor>> = acceptor.clone();
    async move {
      loop {
        let socket = listener.accept().await;
        if let Err(error) = socket {
          println!("Aborted connection: {}", error);
          continue;
        }
        let (stream, addr) = socket.unwrap();
        tokio::spawn(handle_connection(state.clone(), addr, stream, acceptor.clone().lock().unwrap().clone()));
      }
    }
  });

  tokio::signal::ctrl_c().await.unwrap();
  println!("\nAborting...");

  Ok(())
}
