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

async fn handle_connection(peer_map: PeerMap, raw_stream: tokio_native_tls::TlsStream<tokio::net::TcpStream>, addr: std::net::SocketAddr) {
  println!("{}: Connecting...", addr);

  let ws_stream = tokio_tungstenite::accept_async(raw_stream)
      .await
      .expect("Error during the websocket handshake occurred");
  println!("{}: Connection established.", addr);

  let (internal_tx, internal_rx) = futures_channel::mpsc::unbounded::<String>();
  peer_map.lock().unwrap().insert(addr, internal_tx);

  let (mut websocket_tx, websocket_rx) = ws_stream.split();

  let incoming_websocket_messages = websocket_rx.try_for_each(|message| {
    match message {
      tungstenite::protocol::Message::Text(payload) => {
        println!("{}: \"{}\"", addr, payload);
        let peers = peer_map.lock().unwrap();
        let peer_txs = peers.iter().filter(|(peer_addr, _peer_tx)| peer_addr != &&addr).map(|(_peer_addr, peer_tx)| peer_tx);
        for peer_tx in peer_txs {
          peer_tx.unbounded_send(payload.clone()).unwrap();
        }
      },
      tungstenite::protocol::Message::Close(payload) => {
        match payload {
          None => println!("{}: Disconnecting... (no data)", addr),
          Some(data) => println!("{}: Disconnecting... ({}: {})", addr, data.code, data.reason),
        }
      },
      _ => { } // binary, ping, and pong frames are ignored
    }
    futures_util::future::ok(())
  });

  let incoming_peer_messages = internal_rx.for_each(|message| {
    // TODO: we drop the following future ("futures_util::sink::Send") on the floor, which means it never sends
    // but how should that be solved??
    websocket_tx.send(tungstenite::protocol::Message::Text(message.clone()));
    futures_util::future::ready(())
  });

  futures_util::pin_mut!(incoming_websocket_messages, incoming_peer_messages);
  futures_util::future::select(incoming_websocket_messages, incoming_peer_messages).await;

  println!("{}: Disconnected.", &addr);
  peer_map.lock().unwrap().remove(&addr);
}

#[derive(Clone)]
struct Configuration {
  pfx_filename: String,
  server_address: String,
  update_interval: core::time::Duration,
}

async fn read_configuration() -> Configuration {
  let configfile = fs::read_to_string("configuration").unwrap();
  let lines: Vec<&str> = configfile.split('\n').collect();
  Configuration{
    pfx_filename: String::from(lines[0]),
    server_address: String::from(lines[1]),
    update_interval: core::time::Duration::from_secs(lines[2].parse().unwrap()),
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

// TODO: cleaner error handling

#[tokio::main]
async fn main() -> std::io::Result<()> {
  let configuration = Arc::new(read_configuration().await);
  let state = PeerMap::new(Mutex::new(HashMap::new()));
  let listener = tokio::net::TcpListener::bind(&configuration.server_address).await.unwrap();

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
    println!("Listening on: {}", configuration.server_address);
    let acceptor: Arc<Mutex<tokio_native_tls::TlsAcceptor>> = acceptor.clone();
    async move {
      loop {
        let (stream, addr) = listener.accept().await.unwrap();
        let current_acceptor = acceptor.clone().lock().unwrap().clone();
        tokio::spawn(handle_connection(state.clone(), current_acceptor.accept(stream).await.unwrap(), addr));
      }
    }
  });

  tokio::signal::ctrl_c().await.unwrap();
  println!("Aborting...");

  Ok(())
}
