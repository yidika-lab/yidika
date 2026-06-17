use tokio::net::{TcpListener, TcpStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use std::sync::Arc;
use crate::netlib::ServerInstance;
use httparse::Request;
use bytes::BytesMut;

// Constante pour la réponse rapide (benchmark)
const RESPONSE_200: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 16\r\n\r\nHello, benchmark!";

pub async fn run_server(server: ServerInstance, addr: String) {
    let server = Arc::new(server);
    let listener = TcpListener::bind(addr.as_str()).await.unwrap();
    
    println!("🚀 Yidika Hyper Server listening on {}", addr);
    
    // Warm-up pour Windows
    if let Ok(warmup) = TcpStream::connect(addr.as_str()).await {
        drop(warmup);
    }
    
    loop {
        match listener.accept().await {
            Ok((stream, _addr)) => {
                let server = server.clone();
                
                // Spawn une TÂCHE LÉGÈRE (pas un thread OS !)
                tokio::spawn(async move {
                    handle_connection_fast(stream, &server).await;
                });
            }
            Err(e) => {
                eprintln!("Accept error: {}", e);
            }
        }
    }
}

async fn handle_connection_fast(mut stream: TcpStream, _server: &ServerInstance) {
    let mut buf = BytesMut::with_capacity(4096);
    
    loop {
        match stream.read_buf(&mut buf).await {
            Ok(0) => return, // Connexion fermée
            Ok(_) => {
                // Essayer de parser la requête
                let mut headers = [httparse::EMPTY_HEADER; 16];
                let mut req = Request::new(&mut headers);
                
                if let Ok(httparse::Status::Complete(_)) = req.parse(&buf) {
                    // Pour le moment, on renvoie toujours la même réponse rapide
                    // Plus tard : intégrer les routes et handlers Yidika
                    stream.write_all(RESPONSE_200).await.unwrap();
                    stream.flush().await.unwrap();
                    
                    // Réinitialiser le buffer pour keep-alive
                    buf.clear();
                }
            }
            Err(e) => {
                eprintln!("Read error: {}", e);
                return;
            }
        }
    }
}

// Fonction pour démarrer le serveur avec Tokio
pub fn start_hyper_server(server: ServerInstance, addr: String) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    
    rt.block_on(async {
        run_server(server, addr).await;
    });
}
