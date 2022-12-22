use futures_channel::mpsc::UnboundedSender;
use futures_channel::mpsc::unbounded;
use futures_util::{future, pin_mut, stream::TryStreamExt, StreamExt};
use native_tls::Identity;
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::fs;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::Mutex;
//use tokio::io::AsyncRead;
//use tokio::io::AsyncWrite;
use tokio::net::TcpStream;
use tungstenite::protocol::Message;

type Tx = UnboundedSender<Message>;
type PeerMap = Arc<Mutex<HashMap<SocketAddr, Tx>>>;

async fn handle_connection(peer_map: PeerMap, raw_stream: tokio_native_tls::TlsStream<TcpStream>, addr: SocketAddr) {
    println!("Incoming TCP connection from: {}", addr);

    let ws_stream = tokio_tungstenite::accept_async(raw_stream)
        .await
        .expect("Error during the websocket handshake occurred");
    println!("WebSocket connection established: {}", addr);

    // Insert the write part of this peer to the peer map.
    let (tx, rx) = unbounded();
    peer_map.lock().unwrap().insert(addr, tx);

    let (outgoing, incoming) = ws_stream.split();

    let broadcast_incoming = incoming.try_for_each(|msg| {
        println!("Received a message from {}: {}", addr, msg.to_text().unwrap());
        let peers = peer_map.lock().unwrap();

        // We want to broadcast the message to everyone except ourselves.
        let broadcast_recipients =
            peers.iter().filter(|(peer_addr, _)| peer_addr != &&addr).map(|(_, ws_sink)| ws_sink);

        for recp in broadcast_recipients {
            recp.unbounded_send(msg.clone()).unwrap();
        }

        future::ok(())
    });

    let receive_from_others = rx.map(Ok).forward(outgoing);

    pin_mut!(broadcast_incoming, receive_from_others);
    future::select(broadcast_incoming, receive_from_others).await;

    println!("{} disconnected", &addr);
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
  let pkcs12 = Identity::from_pkcs12(&pkcs12, "").unwrap();
  tokio_native_tls::TlsAcceptor::from(native_tls::TlsAcceptor::builder(pkcs12).build().unwrap())
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
  let configuration = Arc::new(read_configuration().await);

  let acceptor: Arc<Mutex<tokio_native_tls::TlsAcceptor>> = Arc::new(Mutex::new(prepare_acceptor(&configuration).await));

  let configuration1: Arc<Configuration> = configuration.clone();
  let acceptor1: Arc<Mutex<tokio_native_tls::TlsAcceptor>> = acceptor.clone();
  tokio::spawn(async move {
    let duration = configuration1.update_interval;
    let mut interval = tokio::time::interval_at(tokio::time::Instant::now() + duration, duration);
    loop {
      interval.tick().await;
      let new_acceptor = prepare_acceptor(&configuration1).await;
      *(acceptor1.lock().unwrap()) = new_acceptor;
    }
  });

  let state = PeerMap::new(Mutex::new(HashMap::new()));

  // Create the event loop and TCP listener we'll accept connections on.
  let listener = tokio::net::TcpListener::bind(&configuration.server_address).await?;
  tokio::spawn(async move {
    // Let's spawn the handling of each connection in a separate task.
    println!("Listening on: {}", configuration.server_address);
    loop {
      let (stream, addr) = listener.accept().await.unwrap();
      let current_acceptor = acceptor.clone().lock().unwrap().clone();
      tokio::spawn(handle_connection(state.clone(), current_acceptor.accept(stream).await.unwrap(), addr));
    }
  });

  tokio::signal::ctrl_c().await.unwrap();
  println!("Aborting...");

  Ok(())
}
