// src/message.rs
use bytes::{Buf, BufMut, BytesMut};
use tokio_util::codec::{Decoder, Encoder};
use std::io;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MessageId {
    Choke = 0,
    Unchoke = 1,
    Interested = 2,
    NotInterested = 3,
    Have = 4,
    Bitfield = 5,
    Request = 6,
    Piece = 7,
    Cancel = 8,
}

#[derive(Debug, Clone)]
pub struct Message {
    pub id: MessageId,
    pub payload: Vec<u8>,
}

impl Message {
    pub fn new(id: MessageId, payload: Vec<u8>) -> Self {
        Self {
            id,
            payload,
        }
    }
}

pub struct PeerCodec {
    // For reading partial messages
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
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        // Need at least 4 bytes for message length
        if src.len() < 4 {
            return Ok(None);
        }

        // Read but don't consume message length
        let mut length_bytes = [0u8; 4];
        length_bytes.copy_from_slice(&src[..4]);
        let length = u32::from_be_bytes(length_bytes) as usize;

        // Check if we have the full message
        if src.len() < 4 + length {
            return Ok(None);
        }

        // Consume message length
        src.advance(4);

        // Handle keepalive message
        if length == 0 {
            return Ok(None);
        }

        // Need at least 1 byte for message ID
        if length < 1 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Message length too small",
            ));
        }

        // Read message ID
        let id = match src[0] {
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
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("Invalid message ID: {}", n),
                ))
            }
        };

        // Consume message ID
        src.advance(1);

        // Extract payload
        let payload = if length > 1 {
            src[..length - 1].to_vec()
        } else {
            vec![]
        };
        src.advance(length - 1);

        Ok(Some(Message { id, payload }))
    }
}

impl Encoder<Message> for PeerCodec {
    type Error = io::Error;

    fn encode(&mut self, item: Message, dst: &mut BytesMut) -> Result<(), Self::Error> {
        // Reserve space
        dst.reserve(4 + 1 + item.payload.len());

        // Write length prefix (payload + 1 byte for message id)
        dst.put_u32(1 + item.payload.len() as u32);

        // Write message ID
        dst.put_u8(item.id as u8);

        // Write payload
        dst.extend_from_slice(&item.payload);

        Ok(())
    }
}
