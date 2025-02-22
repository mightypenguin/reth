#![allow(missing_docs)]

use crate::{error::DecodePacketError, PeerId, MAX_PACKET_SIZE, MIN_PACKET_SIZE};
use enr::{Enr, EnrKey};
use reth_primitives::{
    bytes::{Buf, BufMut, Bytes, BytesMut},
    keccak256,
    rpc_utils::rlp,
    ForkId, NodeRecord, H256,
};
use reth_rlp::{length_of_length, Decodable, DecodeError, Encodable, Header};
use reth_rlp_derive::{RlpDecodable, RlpEncodable};
use secp256k1::{
    ecdsa::{RecoverableSignature, RecoveryId},
    SecretKey, SECP256K1,
};
use std::net::IpAddr;

// Note: this is adapted from https://github.com/vorot93/discv4

/// Id for message variants.
#[derive(Debug)]
#[repr(u8)]
pub enum MessageId {
    Ping = 1,
    Pong = 2,
    FindNode = 3,
    Neighbours = 4,
    EnrRequest = 5,
    EnrResponse = 6,
}

impl MessageId {
    /// Converts the byte that represents the message id to the enum.
    fn from_u8(msg: u8) -> Result<Self, u8> {
        let msg = match msg {
            1 => MessageId::Ping,
            2 => MessageId::Pong,
            3 => MessageId::FindNode,
            4 => MessageId::Neighbours,
            5 => MessageId::EnrRequest,
            6 => MessageId::EnrResponse,
            _ => return Err(msg),
        };
        Ok(msg)
    }
}

/// All message variants
#[derive(Debug, Eq, PartialEq)]
pub enum Message {
    Ping(Ping),
    Pong(Pong),
    FindNode(FindNode),
    Neighbours(Neighbours),
    EnrRequest(EnrRequest),
    EnrResponse(EnrResponse),
}

// === impl Message ===

impl Message {
    /// Returns the id for this type
    pub fn msg_type(&self) -> MessageId {
        match self {
            Message::Ping(_) => MessageId::Ping,
            Message::Pong(_) => MessageId::Pong,
            Message::FindNode(_) => MessageId::FindNode,
            Message::Neighbours(_) => MessageId::Neighbours,
            Message::EnrRequest(_) => MessageId::EnrRequest,
            Message::EnrResponse(_) => MessageId::EnrResponse,
        }
    }

    /// Encodes the UDP datagram, See <https://github.com/ethereum/devp2p/blob/master/discv4.md#wire-protocol>
    ///
    /// The datagram is `header || payload`
    /// where header is `hash || signature || packet-type`
    pub fn encode(&self, secret_key: &SecretKey) -> (Bytes, H256) {
        // allocate max packet size
        let mut datagram = BytesMut::with_capacity(MAX_PACKET_SIZE);

        // since signature has fixed len, we can split and fill the datagram buffer at fixed
        // positions, this way we can encode the message directly in the datagram buffer
        let mut sig_bytes = datagram.split_off(H256::len_bytes());
        let mut payload = sig_bytes.split_off(secp256k1::constants::COMPACT_SIGNATURE_SIZE + 1);

        match self {
            Message::Ping(message) => {
                payload.put_u8(1);
                message.encode(&mut payload);
            }
            Message::Pong(message) => {
                payload.put_u8(2);
                message.encode(&mut payload);
            }
            Message::FindNode(message) => {
                payload.put_u8(3);
                message.encode(&mut payload);
            }
            Message::Neighbours(message) => {
                payload.put_u8(4);
                message.encode(&mut payload);
            }
            Message::EnrRequest(message) => {
                payload.put_u8(5);
                message.encode(&mut payload);
            }
            Message::EnrResponse(message) => {
                payload.put_u8(6);
                message.encode(&mut payload);
            }
        }

        let signature: RecoverableSignature = SECP256K1.sign_ecdsa_recoverable(
            &secp256k1::Message::from_slice(keccak256(&payload).as_ref())
                .expect("is correct MESSAGE_SIZE; qed"),
            secret_key,
        );

        let (rec, sig) = signature.serialize_compact();
        sig_bytes.extend_from_slice(&sig);
        sig_bytes.put_u8(rec.to_i32() as u8);
        sig_bytes.unsplit(payload);

        let hash = keccak256(&sig_bytes);
        datagram.extend_from_slice(hash.as_bytes());

        datagram.unsplit(sig_bytes);
        (datagram.freeze(), hash)
    }

    /// Decodes the [`Message`] from the given buffer.
    ///
    /// Returns the decoded message and the public key of the sender.
    pub fn decode(packet: &[u8]) -> Result<Packet, DecodePacketError> {
        if packet.len() < MIN_PACKET_SIZE {
            return Err(DecodePacketError::PacketTooShort)
        }

        // parses the wire-protocol, every packet starts with a header:
        // packet-header = hash || signature || packet-type
        // hash = keccak256(signature || packet-type || packet-data)
        // signature = sign(packet-type || packet-data)

        let header_hash = keccak256(&packet[32..]);
        let data_hash = H256::from_slice(&packet[..32]);
        if data_hash != header_hash {
            return Err(DecodePacketError::HashMismatch)
        }

        let signature = &packet[32..96];
        let recovery_id = RecoveryId::from_i32(packet[96] as i32)?;
        let recoverable_sig = RecoverableSignature::from_compact(signature, recovery_id)?;

        // recover the public key
        let msg = secp256k1::Message::from_slice(keccak256(&packet[97..]).as_bytes())?;

        let pk = SECP256K1.recover_ecdsa(&msg, &recoverable_sig)?;
        let node_id = PeerId::from_slice(&pk.serialize_uncompressed()[1..]);

        let msg_type = packet[97];
        let payload = &mut &packet[98..];

        let msg = match MessageId::from_u8(msg_type).map_err(DecodePacketError::UnknownMessage)? {
            MessageId::Ping => Message::Ping(Ping::decode(payload)?),
            MessageId::Pong => Message::Pong(Pong::decode(payload)?),
            MessageId::FindNode => Message::FindNode(FindNode::decode(payload)?),
            MessageId::Neighbours => Message::Neighbours(Neighbours::decode(payload)?),
            MessageId::EnrRequest => Message::EnrRequest(EnrRequest::decode(payload)?),
            MessageId::EnrResponse => Message::EnrResponse(EnrResponse::decode(payload)?),
        };

        Ok(Packet { msg, node_id, hash: header_hash })
    }
}

/// Decoded packet
#[derive(Debug)]
pub struct Packet {
    pub msg: Message,
    pub node_id: PeerId,
    pub hash: H256,
}

/// Represents the `from`, `to` fields in the packets
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, RlpEncodable, RlpDecodable)]
pub struct NodeEndpoint {
    pub address: IpAddr,
    pub udp_port: u16,
    pub tcp_port: u16,
}

impl From<NodeRecord> for NodeEndpoint {
    fn from(NodeRecord { address, tcp_port, udp_port, .. }: NodeRecord) -> Self {
        Self { address, tcp_port, udp_port }
    }
}

/// A [FindNode packet](https://github.com/ethereum/devp2p/blob/master/discv4.md#findnode-packet-0x03).
#[derive(Clone, Copy, Debug, Eq, PartialEq, RlpEncodable, RlpDecodable)]
pub struct FindNode {
    pub id: PeerId,
    pub expire: u64,
}

/// A [Neighbours packet](https://github.com/ethereum/devp2p/blob/master/discv4.md#neighbors-packet-0x04).
#[derive(Clone, Debug, Eq, PartialEq, RlpEncodable, RlpDecodable)]
pub struct Neighbours {
    pub nodes: Vec<NodeRecord>,
    pub expire: u64,
}

/// Passthrough newtype to [`Enr`].
///
/// We need to wrap the ENR type because of Rust's orphan rules not allowing
/// implementing a foreign trait on a foreign type.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EnrWrapper<K: EnrKey>(Enr<K>);

impl<K: EnrKey> EnrWrapper<K> {
    pub fn new(enr: Enr<K>) -> Self {
        EnrWrapper(enr)
    }
}

impl<K> Encodable for EnrWrapper<K>
where
    K: EnrKey,
{
    fn encode(&self, out: &mut dyn BufMut) {
        let payload_length = self.0.signature().length() +
            self.0.seq().length() +
            self.0.iter().fold(0, |acc, (k, v)| acc + k.as_slice().length() + v.len());

        let header = Header { list: true, payload_length };
        header.encode(out);

        self.0.signature().encode(out);
        self.0.seq().encode(out);

        for (k, v) in self.0.iter() {
            // Keys are byte data
            k.as_slice().encode(out);
            // Values are raw RLP encoded data
            out.put_slice(v);
        }
    }

    fn length(&self) -> usize {
        let payload_length = self.0.signature().length() +
            self.0.seq().length() +
            self.0.iter().fold(0, |acc, (k, v)| acc + k.as_slice().length() + v.len());
        payload_length + length_of_length(payload_length)
    }
}

impl<K: EnrKey> Decodable for EnrWrapper<K> {
    fn decode(buf: &mut &[u8]) -> Result<Self, DecodeError> {
        let enr = <Enr<K> as rlp::Decodable>::decode(&rlp::Rlp::new(buf))
            .map_err(|e| match e {
                rlp::DecoderError::RlpIsTooShort => DecodeError::InputTooShort,
                rlp::DecoderError::RlpInvalidLength => DecodeError::Overflow,
                rlp::DecoderError::RlpExpectedToBeList => DecodeError::UnexpectedString,
                rlp::DecoderError::RlpExpectedToBeData => DecodeError::UnexpectedList,
                rlp::DecoderError::RlpDataLenWithZeroPrefix |
                rlp::DecoderError::RlpListLenWithZeroPrefix => DecodeError::LeadingZero,
                rlp::DecoderError::RlpInvalidIndirection => DecodeError::NonCanonicalSize,
                rlp::DecoderError::RlpIncorrectListLen => {
                    DecodeError::Custom("incorrect list length when decoding rlp")
                }
                rlp::DecoderError::RlpIsTooBig => DecodeError::Custom("rlp is too big"),
                rlp::DecoderError::RlpInconsistentLengthAndData => {
                    DecodeError::Custom("inconsistent length and data when decoding rlp")
                }
                rlp::DecoderError::Custom(s) => DecodeError::Custom(s),
            })
            .map(EnrWrapper::new);
        if enr.is_ok() {
            // Decode was successful, advance buffer
            let header = Header::decode(buf)?;
            buf.advance(header.payload_length);
        }
        enr
    }
}

/// A [ENRRequest packet](https://github.com/ethereum/devp2p/blob/master/discv4.md#enrrequest-packet-0x05).
#[derive(Clone, Copy, Debug, Eq, PartialEq, RlpEncodable, RlpDecodable)]
pub struct EnrRequest {
    pub expire: u64,
}

/// A [ENRResponse packet](https://github.com/ethereum/devp2p/blob/master/discv4.md#enrresponse-packet-0x06).
#[derive(Clone, Debug, Eq, PartialEq, RlpEncodable)]
pub struct EnrResponse {
    pub request_hash: H256,
    pub enr: EnrWrapper<SecretKey>,
}

// === impl EnrResponse ===

impl EnrResponse {
    /// Returns the [`ForkId`] if set
    ///
    /// See also <https://github.com/ethereum/go-ethereum/blob/9244d5cd61f3ea5a7645fdf2a1a96d53421e412f/eth/protocols/eth/discovery.go#L36>
    pub fn eth_fork_id(&self) -> Option<ForkId> {
        let mut maybe_fork_id = self.enr.0.get(b"eth")?;
        ForkId::decode(&mut maybe_fork_id).ok()
    }
}

impl Decodable for EnrResponse {
    fn decode(buf: &mut &[u8]) -> Result<Self, DecodeError> {
        let b = &mut &**buf;
        let rlp_head = Header::decode(b)?;
        if !rlp_head.list {
            return Err(DecodeError::UnexpectedString)
        }
        // let started_len = b.len();
        let this = Self {
            request_hash: reth_rlp::Decodable::decode(b)?,
            enr: EnrWrapper::<SecretKey>::decode(b)?,
        };
        // TODO: `Decodable` can be derived once we have native reth_rlp decoding for ENR: <https://github.com/paradigmxyz/reth/issues/482>
        // Skipping the size check here is fine since the `buf` is the UDP datagram
        // let consumed = started_len - b.len();
        // if consumed != rlp_head.payload_length {
        //     return Err(reth_rlp::DecodeError::ListLengthMismatch {
        //         expected: rlp_head.payload_length,
        //         got: consumed,
        //     })
        // }
        *buf = *b;
        Ok(this)
    }
}

/// A [Ping packet](https://github.com/ethereum/devp2p/blob/master/discv4.md#ping-packet-0x01).
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Ping {
    pub from: NodeEndpoint,
    pub to: NodeEndpoint,
    pub expire: u64,
    /// Optional enr_seq for <https://eips.ethereum.org/EIPS/eip-868>
    pub enr_sq: Option<u64>,
}

impl Encodable for Ping {
    fn encode(&self, out: &mut dyn BufMut) {
        #[derive(RlpEncodable)]
        struct V4PingMessage<'a> {
            version: u32,
            from: &'a NodeEndpoint,
            to: &'a NodeEndpoint,
            expire: u64,
        }

        #[derive(RlpEncodable)]
        struct V4PingMessageEIP868<'a> {
            version: u32,
            from: &'a NodeEndpoint,
            to: &'a NodeEndpoint,
            expire: u64,
            enr_seq: u64,
        }
        if let Some(enr_seq) = self.enr_sq {
            V4PingMessageEIP868 {
                version: 4, // version 4
                from: &self.from,
                to: &self.to,
                expire: self.expire,
                enr_seq,
            }
            .encode(out);
        } else {
            V4PingMessage {
                version: 4, // version 4
                from: &self.from,
                to: &self.to,
                expire: self.expire,
            }
            .encode(out);
        }
    }
}

impl Decodable for Ping {
    fn decode(buf: &mut &[u8]) -> Result<Self, DecodeError> {
        let b = &mut &**buf;
        let rlp_head = Header::decode(b)?;
        if !rlp_head.list {
            return Err(DecodeError::UnexpectedString)
        }
        let started_len = b.len();
        let _version = u32::decode(b)?;
        let mut this = Self {
            from: Decodable::decode(b)?,
            to: Decodable::decode(b)?,
            expire: Decodable::decode(b)?,
            enr_sq: None,
        };

        // only decode the ENR sequence if there's more data in the datagram to decode else skip
        if b.has_remaining() {
            this.enr_sq = Some(Decodable::decode(b)?);
        }

        let consumed = started_len - b.len();
        if consumed > rlp_head.payload_length {
            return Err(DecodeError::ListLengthMismatch {
                expected: rlp_head.payload_length,
                got: consumed,
            })
        }
        let rem = rlp_head.payload_length - consumed;
        b.advance(rem);
        *buf = *b;
        Ok(this)
    }
}

/// A [Pong packet](https://github.com/ethereum/devp2p/blob/master/discv4.md#pong-packet-0x02).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Pong {
    pub to: NodeEndpoint,
    pub echo: H256,
    pub expire: u64,
    /// Optional enr_seq for <https://eips.ethereum.org/EIPS/eip-868>
    pub enr_sq: Option<u64>,
}

impl Encodable for Pong {
    fn encode(&self, out: &mut dyn BufMut) {
        #[derive(RlpEncodable)]
        struct PongMessageEIP868<'a> {
            to: &'a NodeEndpoint,
            echo: &'a H256,
            expire: u64,
            enr_seq: u64,
        }

        #[derive(RlpEncodable)]
        struct PongMessage<'a> {
            to: &'a NodeEndpoint,
            echo: &'a H256,
            expire: u64,
        }

        if let Some(enr_seq) = self.enr_sq {
            PongMessageEIP868 { to: &self.to, echo: &self.echo, expire: self.expire, enr_seq }
                .encode(out);
        } else {
            PongMessage { to: &self.to, echo: &self.echo, expire: self.expire }.encode(out);
        }
    }
}

impl Decodable for Pong {
    fn decode(buf: &mut &[u8]) -> Result<Self, DecodeError> {
        let b = &mut &**buf;
        let rlp_head = Header::decode(b)?;
        if !rlp_head.list {
            return Err(DecodeError::UnexpectedString)
        }
        let started_len = b.len();
        let mut this = Self {
            to: Decodable::decode(b)?,
            echo: Decodable::decode(b)?,
            expire: Decodable::decode(b)?,
            enr_sq: None,
        };

        // only decode the ENR sequence if there's more data in the datagram to decode else skip
        if b.has_remaining() {
            this.enr_sq = Some(Decodable::decode(b)?);
        }

        let consumed = started_len - b.len();
        if consumed > rlp_head.payload_length {
            return Err(DecodeError::ListLengthMismatch {
                expected: rlp_head.payload_length,
                got: consumed,
            })
        }
        let rem = rlp_head.payload_length - consumed;
        b.advance(rem);
        *buf = *b;

        Ok(this)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        test_utils::{rng_endpoint, rng_ipv4_record, rng_ipv6_record, rng_message},
        SAFE_MAX_DATAGRAM_NEIGHBOUR_RECORDS,
    };
    use enr::{EnrBuilder, EnrPublicKey};
    use rand::{thread_rng, Rng, RngCore};
    use reth_primitives::hex_literal::hex;

    #[test]
    fn test_endpoint_ipv_v4() {
        let mut rng = thread_rng();
        for _ in 0..100 {
            let mut ip = [0u8; 4];
            rng.fill_bytes(&mut ip);
            let msg = NodeEndpoint {
                address: IpAddr::V4(ip.into()),
                tcp_port: rng.gen(),
                udp_port: rng.gen(),
            };

            let mut buf = BytesMut::new();
            msg.encode(&mut buf);

            let decoded = NodeEndpoint::decode(&mut buf.as_ref()).unwrap();
            assert_eq!(msg, decoded);
        }
    }

    #[test]
    fn test_endpoint_ipv_64() {
        let mut rng = thread_rng();
        for _ in 0..100 {
            let mut ip = [0u8; 16];
            rng.fill_bytes(&mut ip);
            let msg = NodeEndpoint {
                address: IpAddr::V6(ip.into()),
                tcp_port: rng.gen(),
                udp_port: rng.gen(),
            };

            let mut buf = BytesMut::new();
            msg.encode(&mut buf);

            let decoded = NodeEndpoint::decode(&mut buf.as_ref()).unwrap();
            assert_eq!(msg, decoded);
        }
    }

    #[test]
    fn test_ping_message() {
        let mut rng = thread_rng();
        for _ in 0..100 {
            let mut ip = [0u8; 16];
            rng.fill_bytes(&mut ip);
            let msg = Ping {
                from: rng_endpoint(&mut rng),
                to: rng_endpoint(&mut rng),
                expire: 0,
                enr_sq: None,
            };

            let mut buf = BytesMut::new();
            msg.encode(&mut buf);

            let decoded = Ping::decode(&mut buf.as_ref()).unwrap();
            assert_eq!(msg, decoded);
        }
    }

    #[test]
    fn test_ping_message_with_enr() {
        let mut rng = thread_rng();
        for _ in 0..100 {
            let mut ip = [0u8; 16];
            rng.fill_bytes(&mut ip);
            let msg = Ping {
                from: rng_endpoint(&mut rng),
                to: rng_endpoint(&mut rng),
                expire: 0,
                enr_sq: Some(rng.gen()),
            };

            let mut buf = BytesMut::new();
            msg.encode(&mut buf);

            let decoded = Ping::decode(&mut buf.as_ref()).unwrap();
            assert_eq!(msg, decoded);
        }
    }

    #[test]
    fn test_pong_message() {
        let mut rng = thread_rng();
        for _ in 0..100 {
            let mut ip = [0u8; 16];
            rng.fill_bytes(&mut ip);
            let msg = Pong {
                to: rng_endpoint(&mut rng),
                echo: H256::random(),
                expire: rng.gen(),
                enr_sq: None,
            };

            let mut buf = BytesMut::new();
            msg.encode(&mut buf);

            let decoded = Pong::decode(&mut buf.as_ref()).unwrap();
            assert_eq!(msg, decoded);
        }
    }

    #[test]
    fn test_pong_message_with_enr() {
        let mut rng = thread_rng();
        for _ in 0..100 {
            let mut ip = [0u8; 16];
            rng.fill_bytes(&mut ip);
            let msg = Pong {
                to: rng_endpoint(&mut rng),
                echo: H256::random(),
                expire: rng.gen(),
                enr_sq: Some(rng.gen()),
            };

            let mut buf = BytesMut::new();
            msg.encode(&mut buf);

            let decoded = Pong::decode(&mut buf.as_ref()).unwrap();
            assert_eq!(msg, decoded);
        }
    }

    #[test]
    fn test_hash_mismatch() {
        let mut rng = thread_rng();
        let msg = rng_message(&mut rng);
        let (secret_key, _) = SECP256K1.generate_keypair(&mut rng);
        let (buf, _) = msg.encode(&secret_key);
        let mut buf = BytesMut::from(buf.as_ref());
        buf.put_u8(0);
        match Message::decode(buf.as_ref()).unwrap_err() {
            DecodePacketError::HashMismatch => {}
            err => {
                unreachable!("unexpected err {}", err)
            }
        }
    }

    #[test]
    fn neighbours_max_ipv4() {
        let mut rng = thread_rng();
        let msg = Message::Neighbours(Neighbours {
            nodes: std::iter::repeat_with(|| rng_ipv4_record(&mut rng)).take(16).collect(),
            expire: rng.gen(),
        });
        let (secret_key, _) = SECP256K1.generate_keypair(&mut rng);

        let (encoded, _) = msg.encode(&secret_key);
        // Assert that 16 nodes never fit into one packet
        assert!(encoded.len() > MAX_PACKET_SIZE, "{} {msg:?}", encoded.len());
    }

    #[test]
    fn neighbours_max_nodes() {
        let mut rng = thread_rng();
        for _ in 0..1000 {
            let msg = Message::Neighbours(Neighbours {
                nodes: std::iter::repeat_with(|| rng_ipv6_record(&mut rng))
                    .take(SAFE_MAX_DATAGRAM_NEIGHBOUR_RECORDS)
                    .collect(),
                expire: rng.gen(),
            });
            let (secret_key, _) = SECP256K1.generate_keypair(&mut rng);

            let (encoded, _) = msg.encode(&secret_key);
            assert!(encoded.len() <= MAX_PACKET_SIZE, "{} {msg:?}", encoded.len());

            let mut neighbours = Neighbours {
                nodes: std::iter::repeat_with(|| rng_ipv6_record(&mut rng))
                    .take(SAFE_MAX_DATAGRAM_NEIGHBOUR_RECORDS - 1)
                    .collect(),
                expire: rng.gen(),
            };
            neighbours.nodes.push(rng_ipv4_record(&mut rng));
            let msg = Message::Neighbours(neighbours);
            let (encoded, _) = msg.encode(&secret_key);
            assert!(encoded.len() <= MAX_PACKET_SIZE, "{} {msg:?}", encoded.len());
        }
    }

    #[test]
    fn test_encode_decode_message() {
        let mut rng = thread_rng();
        for _ in 0..100 {
            let msg = rng_message(&mut rng);
            let (secret_key, pk) = SECP256K1.generate_keypair(&mut rng);
            let sender_id = PeerId::from_slice(&pk.serialize_uncompressed()[1..]);

            let (buf, _) = msg.encode(&secret_key);

            let packet = Message::decode(buf.as_ref()).unwrap();

            assert_eq!(msg, packet.msg);
            assert_eq!(sender_id, packet.node_id);
        }
    }

    #[test]
    fn decode_pong_packet() {
        let packet = "2ad84c37327a06c2522cf7bc039621da89f68907441b755935bb308dc4cd17d6fe550e90329ad6a516ca7db18e08900067928a0dfa3b5c75d55a42c984497373698d98616662c048983ea85895ea2da765eabeb15525478384e106337bfd8ed50002f3c9843ed8cae682fd1c80a008ad4dead0922211df47593e7d837b2b23d13954285871ca23250ea594993ded84635690e5829670";
        let data = hex::decode(packet).unwrap();
        Message::decode(&data).unwrap();
    }
    #[test]
    fn decode_ping_packet() {
        let packet = "05ae5bf922cf2a93f97632a4ab0943dc252a0dab0c42d86dd62e5d91e1a0966e9b628fbf4763fdfbb928540460b797e6be2e7058a82f6083f6d2e7391bb021741459976d4152aa16bbee0c3609dcfac6668db1ef78b7ee9f8b4ced10dd5ae2900101df04cb8403d12d4f82765f82765fc9843ed8cae6828aa6808463569916829670";
        let data = hex::decode(packet).unwrap();
        Message::decode(&data).unwrap();
    }

    // test vector from the enr library rlp encoding tests
    // <https://github.com/sigp/enr/blob/e59dcb45ea07e423a7091d2a6ede4ad6d8ef2840/src/lib.rs#L1019>

    #[test]
    fn encode_known_rlp_enr() {
        use self::EnrWrapper;
        use enr::{secp256k1::SecretKey, EnrPublicKey};
        use reth_rlp::Decodable;
        use std::net::Ipv4Addr;

        let valid_record =
    hex!("f884b8407098ad865b00a582051940cb9cf36836572411a47278783077011599ed5cd16b76f2635f4e234738f30813a89eb9137e3e3df5266e3a1f11df72ecf1145ccb9c01826964827634826970847f00000189736563703235366b31a103ca634cae0d49acb401d8a4c6b6fe8c55b70d115bf400769cc1400f3258cd31388375647082765f"
    );
        let signature =
    hex!("7098ad865b00a582051940cb9cf36836572411a47278783077011599ed5cd16b76f2635f4e234738f30813a89eb9137e3e3df5266e3a1f11df72ecf1145ccb9c"
    );
        let expected_pubkey =
            hex!("03ca634cae0d49acb401d8a4c6b6fe8c55b70d115bf400769cc1400f3258cd3138");

        let enr = EnrWrapper::<SecretKey>::decode(&mut &valid_record[..]).unwrap();
        let pubkey = enr.0.public_key().encode();

        assert_eq!(enr.0.ip4(), Some(Ipv4Addr::new(127, 0, 0, 1)));
        assert_eq!(enr.0.id(), Some(String::from("v4")));
        assert_eq!(enr.0.udp4(), Some(30303));
        assert_eq!(enr.0.tcp4(), None);
        assert_eq!(enr.0.signature(), &signature[..]);
        assert_eq!(pubkey.to_vec(), expected_pubkey);
        assert!(enr.0.verify());

        let mut encoded = BytesMut::new();
        enr.encode(&mut encoded);
        assert_eq!(&encoded[..], &valid_record[..]);

        // ensure the length is equal
        assert_eq!(enr.length(), valid_record.len());
    }

    // test vector from the enr library rlp encoding tests
    // <https://github.com/sigp/enr/blob/e59dcb45ea07e423a7091d2a6ede4ad6d8ef2840/src/lib.rs#L1019>
    #[test]
    fn decode_enr_rlp() {
        use enr::secp256k1::SecretKey;
        use std::net::Ipv4Addr;

        let valid_record = hex!("f884b8407098ad865b00a582051940cb9cf36836572411a47278783077011599ed5cd16b76f2635f4e234738f30813a89eb9137e3e3df5266e3a1f11df72ecf1145ccb9c01826964827634826970847f00000189736563703235366b31a103ca634cae0d49acb401d8a4c6b6fe8c55b70d115bf400769cc1400f3258cd31388375647082765f");
        let signature = hex!("7098ad865b00a582051940cb9cf36836572411a47278783077011599ed5cd16b76f2635f4e234738f30813a89eb9137e3e3df5266e3a1f11df72ecf1145ccb9c");
        let expected_pubkey =
            hex!("03ca634cae0d49acb401d8a4c6b6fe8c55b70d115bf400769cc1400f3258cd3138");

        let mut valid_record_buf = valid_record.as_slice();
        let enr = EnrWrapper::<SecretKey>::decode(&mut valid_record_buf).unwrap();
        let pubkey = enr.0.public_key().encode();

        // Byte array must be consumed after enr has finished decoding
        assert!(valid_record_buf.is_empty());

        assert_eq!(enr.0.ip4(), Some(Ipv4Addr::new(127, 0, 0, 1)));
        assert_eq!(enr.0.id(), Some(String::from("v4")));
        assert_eq!(enr.0.udp4(), Some(30303));
        assert_eq!(enr.0.tcp4(), None);
        assert_eq!(enr.0.signature(), &signature[..]);
        assert_eq!(pubkey.to_vec(), expected_pubkey);
        assert!(enr.0.verify());
    }

    // test vector from the enr library rlp encoding tests
    // <https://github.com/sigp/enr/blob/e59dcb45ea07e423a7091d2a6ede4ad6d8ef2840/src/lib.rs#LL1206C35-L1206C35>
    #[test]
    fn encode_decode_enr_rlp() {
        use enr::{secp256k1::SecretKey, EnrPublicKey};
        use std::net::Ipv4Addr;

        let key = SecretKey::new(&mut rand::rngs::OsRng);
        let ip = Ipv4Addr::new(127, 0, 0, 1);
        let tcp = 3000;

        let enr = {
            let mut builder = EnrBuilder::new("v4");
            builder.ip(ip.into());
            builder.tcp4(tcp);
            EnrWrapper::new(builder.build(&key).unwrap())
        };

        let mut encoded = BytesMut::new();
        enr.encode(&mut encoded);
        let mut encoded_bytes = &encoded[..];
        let decoded_enr = EnrWrapper::<SecretKey>::decode(&mut encoded_bytes).unwrap();

        // Byte array must be consumed after enr has finished decoding
        assert!(encoded_bytes.is_empty());

        assert_eq!(decoded_enr, enr);
        assert_eq!(decoded_enr.0.id(), Some("v4".into()));
        assert_eq!(decoded_enr.0.ip4(), Some(ip));
        assert_eq!(decoded_enr.0.tcp4(), Some(tcp));
        assert_eq!(decoded_enr.0.public_key().encode(), key.public().encode());
        assert!(decoded_enr.0.verify());
    }
}
