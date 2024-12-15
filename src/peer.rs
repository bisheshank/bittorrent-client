use crate::{
    error::{BitTorrentError, Result},
    message::{Message, MessageId},
    piece::PieceInfo,
    utils::bit_set,
    BLOCK_SIZE,
};
use bytes::{Buf, BufMut, BytesMut};
use futures_util::{SinkExt, StreamExt};
use std::net::{SocketAddr, SocketAddrV4};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_util::codec::{Decoder, Encoder, Framed};

const PROTOCOL: &[u8] = b"BitTorrent protocol";

#[derive(Debug)]
struct Handshake {
    pstrlen: u8,
    pstr: [u8; 19],
    reserved: [u8; 8],
    info_hash: [u8; 20],
    peer_id: [u8; 20],
}

impl Handshake {
    fn new(info_hash: [u8; 20], peer_id: [u8; 20]) -> Self {
        Self {
            pstrlen: 19,
            pstr: *b"BitTorrent protocol",
            reserved: [0; 8],
            info_hash,
            peer_id,
        }
    }

    async fn write_to(&self, stream: &mut TcpStream) -> Result<()> {
        stream.write_u8(self.pstrlen).await?;
        stream.write_all(&self.pstr).await?;
        stream.write_all(&self.reserved).await?;
        stream.write_all(&self.info_hash).await?;
        stream.write_all(&self.peer_id).await?;
        Ok(())
    }

    async fn read_from(stream: &mut TcpStream) -> Result<Self> {
        let pstrlen = stream.read_u8().await?;
        if pstrlen != 19 {
            return Err(BitTorrentError::Protocol("Invalid pstrlen".into()));
        }

        let mut pstr = [0u8; 19];
        stream.read_exact(&mut pstr).await?;
        if &pstr != PROTOCOL {
            return Err(BitTorrentError::Protocol("Invalid protocol string".into()));
        }

        let mut reserved = [0u8; 8];
        stream.read_exact(&mut reserved).await?;

        let mut info_hash = [0u8; 20];
        stream.read_exact(&mut info_hash).await?;

        let mut peer_id = [0u8; 20];
        stream.read_exact(&mut peer_id).await?;

        Ok(Self {
            pstrlen,
            pstr,
            reserved,
            info_hash,
            peer_id,
        })
    }
}

#[derive(Debug)]
pub struct Peer {
    addr: SocketAddrV4,
    stream: Framed<TcpStream, PeerCodec>,
    bitfield: Vec<u8>,
    am_choking: bool,
    am_interested: bool,
    peer_choking: bool,
    peer_interested: bool,
}

impl Peer {
    pub async fn connect(
        addr: SocketAddrV4,
        info_hash: [u8; 20],
        peer_id: [u8; 20],
    ) -> Result<Self> {
        println!("Initiating connection to peer: {}", addr);
        let mut stream = TcpStream::connect(addr).await?;

        println!("Connected to peer: {} - Performing handshake", addr);
        let handshake = Handshake::new(info_hash, peer_id);
        handshake.write_to(&mut stream).await?;

        let received = Handshake::read_from(&mut stream).await?;
        if received.info_hash != info_hash {
            return Err(BitTorrentError::Protocol("Info hash mismatch".into()));
        }

        // Now switch to message protocol
        let mut framed = Framed::new(stream, PeerCodec::new());

        // Wait for bitfield message
        let bitfield_msg = framed
            .next()
            .await
            .ok_or_else(|| BitTorrentError::Peer("Peer disconnected before bitfield".into()))??;

        let bitfield = if bitfield_msg.id == MessageId::Bitfield {
            bitfield_msg.payload
        } else {
            Vec::new() // Some peers might not send a bitfield
        };

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

    pub async fn request_piece(&mut self, piece: &PieceInfo) -> Result<Vec<u8>> {
        println!(
            "Requesting piece {} ({} bytes) from peer {}",
            piece.index(),
            piece.length(),
            self.addr
        );

        if !self.has_piece(piece.index()) {
            return Err(BitTorrentError::Peer("Peer doesn't have piece".into()));
        }

        // Express interest if we haven't already
        if !self.am_interested {
            self.stream
                .send(Message::new(MessageId::Interested, Vec::new()))
                .await?;
            self.am_interested = true;
        }

        // Wait to be unchoked
        while self.peer_choking {
            let msg = self.stream.next().await.ok_or_else(|| {
                BitTorrentError::Peer("Peer disconnected while waiting for unchoke".into())
            })??;

            self.handle_message(msg).await?;
        }

        let mut data = Vec::with_capacity(piece.length());
        let mut offset = 0;

        while offset < piece.length() {
            let block_size = std::cmp::min(BLOCK_SIZE, piece.length() - offset);

            // Send request
            let mut request = vec![0u8; 12];
            (&mut request[0..4]).put_u32(piece.index() as u32);
            (&mut request[4..8]).put_u32(offset as u32);
            (&mut request[8..12]).put_u32(block_size as u32);

            self.stream
                .send(Message::new(MessageId::Request, request))
                .await?;

            // Wait for piece data
            loop {
                let msg = self.stream.next().await.ok_or_else(|| {
                    BitTorrentError::Peer("Peer disconnected while requesting piece".into())
                })??;

                if msg.id == MessageId::Piece {
                    if msg.payload.len() < 8 {
                        return Err(BitTorrentError::Protocol("Invalid piece message".into()));
                    }

                    let index = (&msg.payload[0..4]).get_u32() as usize;
                    let begin = (&msg.payload[4..8]).get_u32() as usize;

                    if index != piece.index() || begin != offset {
                        continue; // Wrong piece or offset, ignore
                    }

                    let block_data = &msg.payload[8..];
                    if block_data.len() != block_size {
                        return Err(BitTorrentError::Protocol("Invalid block size".into()));
                    }

                    data.extend_from_slice(block_data);
                    offset += block_size;
                    break;
                } else {
                    self.handle_message(msg).await?;
                }
            }
        }

        Ok(data)
    }

    pub fn has_piece(&self, index: usize) -> bool {
        bit_set(&self.bitfield, index)
    }

    pub fn get_bitfield(&self) -> Option<&[u8]> {
        if self.bitfield.is_empty() {
            None
        } else {
            Some(&self.bitfield)
        }
    }

    async fn handle_message(&mut self, message: Message) -> Result<()> {
        match message.id {
            MessageId::Choke => self.peer_choking = true,
            MessageId::Unchoke => self.peer_choking = false,
            MessageId::Interested => self.peer_interested = true,
            MessageId::NotInterested => self.peer_interested = false,
            MessageId::Have => {
                if message.payload.len() != 4 {
                    return Err(BitTorrentError::Protocol("Invalid have message".into()));
                }
                let piece_index = (&message.payload[0..4]).get_u32() as usize;
                if piece_index / 8 < self.bitfield.len() {
                    self.bitfield[piece_index / 8] |= 1 << (7 - (piece_index % 8));
                }
            }
            MessageId::Bitfield => {
                if !self.bitfield.is_empty() {
                    return Err(BitTorrentError::Protocol(
                        "Received duplicate bitfield".into(),
                    ));
                }
                self.bitfield = message.payload;
            }
            // Ignore other messages
            _ => {}
        }
        Ok(())
    }

    pub fn addr(&self) -> SocketAddrV4 {
        self.addr
    }
}

#[derive(Debug)]
pub struct PeerCodec {
    partial_msg: BytesMut,
}

impl PeerCodec {
    pub fn new() -> Self {
        Self {
            partial_msg: BytesMut::new(),
        }
    }
}

impl Decoder for PeerCodec {
    type Item = Message;
    type Error = BitTorrentError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>> {
        // Normal messages start with 4-byte length
        if src.len() < 4 {
            return Ok(None);
        }

        let mut length_bytes = [0u8; 4];
        length_bytes.copy_from_slice(&src[..4]);
        let length = u32::from_be_bytes(length_bytes) as usize;

        // Check if we have the full message
        if src.len() < 4 + length {
            return Ok(None);
        }

        src.advance(4); // Consume length

        // Handle keepalive message
        if length == 0 {
            return Ok(None);
        }

        let msg_type = src[0];
        src.advance(1);

        let id = match msg_type {
            0 => MessageId::Choke,
            1 => MessageId::Unchoke,
            2 => MessageId::Interested,
            3 => MessageId::NotInterested,
            4 => MessageId::Have,
            5 => MessageId::Bitfield,
            6 => MessageId::Request,
            7 => MessageId::Piece,
            8 => MessageId::Cancel,
            n => {
                return Err(BitTorrentError::Protocol(format!(
                    "Unknown message type: {}",
                    n
                )))
            }
        };

        let payload = if length > 1 {
            src.split_to(length - 1).to_vec()
        } else {
            vec![]
        };

        Ok(Some(Message::new(id, payload)))
    }
}

impl Encoder<Message> for PeerCodec {
    type Error = BitTorrentError;

    fn encode(&mut self, item: Message, dst: &mut BytesMut) -> Result<()> {
        let msg_len = 1 + item.payload.len(); // 1 byte for message ID
        dst.extend_from_slice(&(msg_len as u32).to_be_bytes());
        dst.put_u8(item.id as u8);
        dst.extend_from_slice(&item.payload);
        Ok(())
    }
}
