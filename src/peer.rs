// src/peer.rs

use tokio::time::timeout;
use std::time::Duration;

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const PEER_READ_TIMEOUT: Duration = Duration::from_secs(5);

impl Peer {
    pub async fn connect(
        addr: SocketAddrV4,
        info_hash: [u8; 20],
        peer_id: [u8; 20],
    ) -> Result<Self> {
        println!("Initiating connection to peer: {}", addr);
        
        // Set timeout for the entire connection process
        let stream = match timeout(HANDSHAKE_TIMEOUT, TcpStream::connect(addr)).await {
            Ok(Ok(stream)) => stream,
            Ok(Err(e)) => return Err(BitTorrentError::Peer(format!("Failed to connect to {}: {}", addr, e))),
            Err(_) => return Err(BitTorrentError::Peer(format!("Connection timeout to {}", addr))),
        };

        // Set TCP keepalive and timeouts
        stream.set_nodelay(true)?;
        stream.set_keepalive(Some(Duration::from_secs(30)))?;
        
        let mut stream = stream;
        println!("Connected to peer: {} - Performing handshake", addr);

        // Perform handshake with timeout
        let handshake_result = timeout(HANDSHAKE_TIMEOUT, async {
            // Send handshake
            let handshake = Handshake::new(info_hash, peer_id);
            handshake.write_to(&mut stream).await?;

            // Receive handshake
            let received = Handshake::read_from(&mut stream).await?;
            if received.info_hash != info_hash {
                return Err(BitTorrentError::Protocol("Info hash mismatch".into()));
            }
            Ok::<_, BitTorrentError>(())
        }).await;

        match handshake_result {
            Ok(Ok(())) => {
                println!("Successfully completed handshake with peer: {}", addr);
            },
            Ok(Err(e)) => {
                return Err(BitTorrentError::Peer(format!("Handshake failed with {}: {}", addr, e)));
            },
            Err(_) => {
                return Err(BitTorrentError::Peer(format!("Handshake timeout with {}", addr)));
            }
        }

        // Switch to message protocol
        let mut framed = Framed::new(stream, PeerCodec::new());

        // Wait for bitfield message with timeout
        let bitfield_result = timeout(PEER_READ_TIMEOUT, async {
            while let Some(msg_result) = framed.next().await {
                match msg_result {
                    Ok(msg) => {
                        match msg.id {
                            MessageId::Bitfield => {
                                return Ok(msg.payload);
                            }
                            // Some peers might not send a bitfield if they have no pieces
                            _ => {
                                if msg.id == MessageId::Have {
                                    return Ok(Vec::new());
                                }
                            }
                        }
                    }
                    Err(e) => {
                        return Err(BitTorrentError::Peer(format!("Failed to read message: {}", e)));
                    }
                }
            }
            Err(BitTorrentError::Peer("Peer disconnected before bitfield".into()))
        }).await;

        let bitfield = match bitfield_result {
            Ok(Ok(bitfield)) => bitfield,
            Ok(Err(e)) => return Err(e),
            Err(_) => return Err(BitTorrentError::Peer(format!("Timeout waiting for bitfield from {}", addr))),
        };

        println!("Successfully established protocol with peer: {}", addr);

        Ok(Self {
            addr,
            stream: framed,
            bitfield,
            am_choking: true,
            am_interested: false,
            peer_choking: true,
            peer_interested: false,
        })
    }
}
