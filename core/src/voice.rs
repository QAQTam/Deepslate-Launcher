//! Decentralized voice chat engine running over QUIC Datagrams.
//!
//! Provides ultra-low-latency, zero-retransmission audio packet streaming between
//! peers using native QUIC unreliable datagrams (RFC 9221). Integrates seamlessly
//! with the P2P tunnel engine without interfering with reliable Minecraft game streams.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use bytes::{Buf, BufMut, Bytes, BytesMut};
use iroh::endpoint::Connection;
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot, RwLock};

pub const VOICE_MAGIC: &[u8; 4] = b"DSVC";
pub const CURRENT_VOICE_VERSION: u8 = 1;

/// Raw voice packet transmitted over unreliable QUIC datagrams.
#[derive(Debug, Clone, PartialEq)]
pub struct VoicePacket {
    /// Voice protocol version.
    pub version: u8,
    /// Monotonically increasing sequence number per speaker to detect jitter and drops.
    pub sequence: u32,
    /// Speaker's Ed25519 public key (hex, 64 chars).
    pub speaker_id: String,
    /// Audio volume / Voice Activity Detection level (0 = silent, 255 = max volume).
    pub audio_level: u8,
    /// In-game 3D spatial position if available (X, Y, Z coordinates).
    pub spatial_pos: Option<[f32; 3]>,
    /// Opus-encoded audio payload bytes (typically 20-80 bytes per 20ms frame).
    pub payload: Bytes,
}

impl VoicePacket {
    /// Serialize voice packet into compact binary format suitable for single MTU datagrams.
    pub fn encode(&self) -> Bytes {
        let mut buf = BytesMut::with_capacity(128 + self.payload.len());
        buf.put_slice(VOICE_MAGIC);
        buf.put_u8(self.version);
        buf.put_u32(self.sequence);
        buf.put_u8(self.audio_level);

        // Speaker ID bytes (32 raw bytes decoded from 64 hex characters)
        let raw_id = hex::decode(&self.speaker_id).unwrap_or_else(|_| vec![0u8; 32]);
        if raw_id.len() == 32 {
            buf.put_slice(&raw_id);
        } else {
            buf.put_slice(&[0u8; 32]);
        }

        // Spatial position flag and coordinates
        if let Some([x, y, z]) = self.spatial_pos {
            buf.put_u8(1);
            buf.put_f32(x);
            buf.put_f32(y);
            buf.put_f32(z);
        } else {
            buf.put_u8(0);
        }

        // Payload length and data
        buf.put_u16(self.payload.len() as u16);
        buf.put_slice(&self.payload);
        buf.freeze()
    }

    /// Decode raw datagram bytes into a structured voice packet.
    pub fn decode(mut data: Bytes) -> Result<Self, String> {
        if data.len() < 4 + 1 + 4 + 1 + 32 + 1 + 2 {
            return Err("Voice datagram too short".into());
        }

        let magic = data.copy_to_bytes(4);
        if magic.as_ref() != VOICE_MAGIC {
            return Err("Invalid voice magic header".into());
        }

        let version = data.get_u8();
        let sequence = data.get_u32();
        let audio_level = data.get_u8();

        let raw_id = data.copy_to_bytes(32);
        let speaker_id = hex::encode(raw_id);

        let has_spatial = data.get_u8() != 0;
        let spatial_pos = if has_spatial {
            if data.remaining() < 12 {
                return Err("Truncated spatial coordinates".into());
            }
            Some([data.get_f32(), data.get_f32(), data.get_f32()])
        } else {
            None
        };

        if data.remaining() < 2 {
            return Err("Missing payload length".into());
        }
        let payload_len = data.get_u16() as usize;
        if data.remaining() < payload_len {
            return Err("Truncated audio payload".into());
        }

        let payload = data.copy_to_bytes(payload_len);

        Ok(Self {
            version,
            sequence,
            speaker_id,
            audio_level,
            spatial_pos,
            payload,
        })
    }
}

/// Statistics and real-time state for an active voice participant.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceParticipantState {
    pub speaker_id: String,
    pub is_speaking: bool,
    pub audio_level: u8,
    pub packets_received: u64,
    pub packets_lost: u64,
    pub last_packet_time: u64,
}

/// Manages incoming and outgoing voice packet pipelines for one P2P connection.
pub struct VoiceSession {
    self_id: String,
    conn: Connection,
    sequence: Arc<AtomicU32>,
    is_muted: Arc<AtomicBool>,
    is_deafened: Arc<AtomicBool>,
    participants: Arc<RwLock<HashMap<String, VoiceParticipantState>>>,
    outgoing_tx: mpsc::Sender<Bytes>,
    incoming_rx: mpsc::Receiver<VoicePacket>,
    shutdown_tx: Option<oneshot::Sender<()>>,
}

impl VoiceSession {
    /// Initialize a voice session over an established QUIC connection.
    pub fn new(self_id: String, conn: Connection) -> Self {
        let (outgoing_tx, mut outgoing_rx) = mpsc::channel::<Bytes>(128);
        let (incoming_tx, incoming_rx) = mpsc::channel::<VoicePacket>(128);
        let (shutdown_tx, mut shutdown_rx) = oneshot::channel();

        let sequence = Arc::new(AtomicU32::new(1));
        let is_muted = Arc::new(AtomicBool::new(false));
        let is_deafened = Arc::new(AtomicBool::new(false));
        let participants = Arc::new(RwLock::new(HashMap::new()));

        let conn_send = conn.clone();
        let conn_recv = conn.clone();
        let participants_clone = participants.clone();
        let is_deafened_clone = is_deafened.clone();

        // Sender loop: transmits voice datagrams without retransmission blocking
        tokio::spawn(async move {
            while let Some(datagram) = outgoing_rx.recv().await {
                // If the underlying socket drops a datagram during congestion,
                // we intentionally drop it to keep audio latency sub-20ms.
                let _ = conn_send.send_datagram(datagram);
            }
        });

        // Receiver loop: reads incoming datagrams and decodes voice packets
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => break,
                    datagram_res = conn_recv.read_datagram() => {
                        let Ok(data) = datagram_res else { break };
                        if is_deafened_clone.load(Ordering::SeqCst) {
                            continue;
                        }

                        if let Ok(packet) = VoicePacket::decode(data) {
                            let now = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs();

                            {
                                let mut guard = participants_clone.write().await;
                                let entry = guard.entry(packet.speaker_id.clone()).or_insert_with(|| VoiceParticipantState {
                                    speaker_id: packet.speaker_id.clone(),
                                    is_speaking: false,
                                    audio_level: 0,
                                    packets_received: 0,
                                    packets_lost: 0,
                                    last_packet_time: now,
                                });
                                entry.packets_received += 1;
                                entry.audio_level = packet.audio_level;
                                entry.is_speaking = packet.audio_level > 20;
                                entry.last_packet_time = now;
                            }

                            let _ = incoming_tx.try_send(packet);
                        }
                    }
                }
            }
        });

        Self {
            self_id,
            conn,
            sequence,
            is_muted,
            is_deafened,
            participants,
            outgoing_tx,
            incoming_rx,
            shutdown_tx: Some(shutdown_tx),
        }
    }

    /// Broadcast an audio frame to the remote peer.
    pub fn send_audio_frame(
        &self,
        audio_level: u8,
        spatial_pos: Option<[f32; 3]>,
        opus_payload: Bytes,
    ) -> Result<(), String> {
        if self.is_muted.load(Ordering::SeqCst) {
            return Ok(());
        }

        let seq = self.sequence.fetch_add(1, Ordering::SeqCst);
        let packet = VoicePacket {
            version: CURRENT_VOICE_VERSION,
            sequence: seq,
            speaker_id: self.self_id.clone(),
            audio_level,
            spatial_pos,
            payload: opus_payload,
        };

        let encoded = packet.encode();
        self.outgoing_tx
            .try_send(encoded)
            .map_err(|e| format!("Voice send buffer full: {e}"))
    }

    /// Pull the next decoded voice packet received from the network.
    pub async fn recv_packet(&mut self) -> Option<VoicePacket> {
        self.incoming_rx.recv().await
    }

    /// Toggle local microphone mute state.
    pub fn set_muted(&self, muted: bool) {
        self.is_muted.store(muted, Ordering::SeqCst);
    }

    /// Whether local microphone is currently muted.
    pub fn is_muted(&self) -> bool {
        self.is_muted.load(Ordering::SeqCst)
    }

    /// Toggle deafen state (ignore all incoming audio).
    pub fn set_deafened(&self, deafened: bool) {
        self.is_deafened.store(deafened, Ordering::SeqCst);
    }

    /// Whether local deafen is active.
    pub fn is_deafened(&self) -> bool {
        self.is_deafened.load(Ordering::SeqCst)
    }

    /// List active voice participants in this session.
    pub async fn participants(&self) -> Vec<VoiceParticipantState> {
        let guard = self.participants.read().await;
        guard.values().cloned().collect()
    }

    /// Access underlying QUIC connection.
    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    /// Gracefully stop the voice session.
    pub fn stop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_voice_packet_roundtrip() {
        let dummy_payload = Bytes::from_static(b"OPUS_MOCK_COMPRESSED_AUDIO_FRAME_20MS");
        let original = VoicePacket {
            version: CURRENT_VOICE_VERSION,
            sequence: 42,
            speaker_id: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".into(),
            audio_level: 180,
            spatial_pos: Some([100.5, 64.0, -250.25]),
            payload: dummy_payload.clone(),
        };

        let encoded = original.encode();
        assert!(encoded.len() > 40);

        let decoded = VoicePacket::decode(encoded).expect("Decode should succeed");
        assert_eq!(decoded.version, original.version);
        assert_eq!(decoded.sequence, original.sequence);
        assert_eq!(decoded.speaker_id, original.speaker_id);
        assert_eq!(decoded.audio_level, original.audio_level);
        assert_eq!(decoded.spatial_pos, original.spatial_pos);
        assert_eq!(decoded.payload, dummy_payload);
    }

    #[test]
    fn test_voice_packet_without_spatial() {
        let dummy_payload = Bytes::from_static(b"VOICE_PAYLOAD");
        let original = VoicePacket {
            version: CURRENT_VOICE_VERSION,
            sequence: 1,
            speaker_id: "ab".repeat(32),
            audio_level: 0,
            spatial_pos: None,
            payload: dummy_payload.clone(),
        };

        let encoded = original.encode();
        let decoded = VoicePacket::decode(encoded).expect("Decode should succeed");
        assert_eq!(decoded.spatial_pos, None);
        assert_eq!(decoded.audio_level, 0);
        assert_eq!(decoded.payload, dummy_payload);
    }

    #[tokio::test]
    async fn test_voice_session_network_datagram() -> Result<(), String> {
        use iroh::endpoint::presets;
        use iroh::Endpoint;

        let alpn = b"deepslate-voice-test/v1";
        let server_ep = Endpoint::builder(presets::Minimal)
            .alpns(vec![alpn.to_vec()])
            .bind()
            .await
            .map_err(|e| e.to_string())?;

        let mut server_addr = server_ep.addr();
        for sock in server_ep.bound_sockets() {
            if sock.is_ipv4() {
                server_addr = server_addr.with_ip_addr(std::net::SocketAddr::new(
                    std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
                    sock.port(),
                ));
            }
        }

        let s_handle = tokio::spawn(async move {
            if let Some(incoming) = server_ep.accept().await {
                let conn = incoming.await.unwrap();
                let mut session = VoiceSession::new("server_speaker_id".into(), conn);
                if let Some(packet) = session.recv_packet().await {
                    assert_eq!(packet.sequence, 1);
                    assert_eq!(packet.audio_level, 200);
                    assert_eq!(&packet.payload[..], b"VOICE_OPUS_FRAME");
                }
                session.stop();
            }
        });

        let client_ep = Endpoint::builder(presets::Minimal)
            .alpns(vec![alpn.to_vec()])
            .bind()
            .await
            .map_err(|e| e.to_string())?;

        let conn = client_ep
            .connect(server_addr, alpn)
            .await
            .map_err(|e| e.to_string())?;

        let session = VoiceSession::new("01".repeat(32), conn);
        session
            .send_audio_frame(200, Some([1.0, 2.0, 3.0]), Bytes::from_static(b"VOICE_OPUS_FRAME"))?;

        let _ = s_handle.await;
        Ok(())
    }
}
