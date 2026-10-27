//! Small vanilla status ping used by the enhanced launcher's saved-server list.

use serde_json::Value;
use std::io;
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{lookup_host, TcpStream};

const MAX_PACKET: usize = 1024 * 1024;

#[derive(Debug, Clone)]
pub struct ServerPing {
    pub version: Option<String>,
    pub motd: String,
    pub online: Option<u32>,
    pub max: Option<u32>,
    pub latency_ms: u128,
}

/// Query a Java server's status endpoint. The address may include `:port`;
/// without one, the standard Minecraft port 25565 is used.
pub async fn ping(address: &str) -> Result<ServerPing, String> {
    tokio::time::timeout(Duration::from_secs(5), ping_inner(address))
        .await
        .map_err(|_| "服务器状态查询超时".to_string())?
}

async fn ping_inner(address: &str) -> Result<ServerPing, String> {
    let (host, port) = parse_address(address)?;
    let socket = lookup_host((host.as_str(), port))
        .await
        .map_err(|e| format!("DNS 查询失败: {e}"))?
        .next()
        .ok_or_else(|| "服务器地址没有可用 IP".to_string())?;
    let mut stream = TcpStream::connect(socket)
        .await
        .map_err(|e| format!("无法连接服务器: {e}"))?;
    stream
        .set_nodelay(true)
        .map_err(|e| format!("设置 TCP 选项失败: {e}"))?;

    let started = Instant::now();
    send_handshake(&mut stream, &host, port).await?;
    stream
        .write_all(&[1, 0])
        .await
        .map_err(|e| format!("发送状态请求失败: {e}"))?;
    let status_packet = read_packet(&mut stream).await?;
    let json = parse_status_packet(&status_packet)?;
    let status_latency = started.elapsed().as_millis();

    // The optional ping/pong exchange is the closest match to the latency shown
    // by Minecraft's multiplayer screen. Keep the status response if a server
    // closes the connection immediately after replying to the status request.
    let latency_ms = match send_ping(&mut stream).await {
        Ok(()) => {
            match tokio::time::timeout(Duration::from_millis(750), read_pong(&mut stream, started))
                .await
            {
                Ok(Ok(latency)) => latency,
                _ => status_latency,
            }
        }
        Err(_) => status_latency,
    };

    let players = json.get("players");
    Ok(ServerPing {
        version: json
            .pointer("/version/name")
            .and_then(Value::as_str)
            .map(str::to_string),
        motd: json
            .get("description")
            .map(component_text)
            .unwrap_or_default(),
        online: players
            .and_then(|v| v.get("online"))
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok()),
        max: players
            .and_then(|v| v.get("max"))
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok()),
        latency_ms,
    })
}

fn parse_address(address: &str) -> Result<(String, u16), String> {
    let address = address.trim();
    if address.is_empty()
        || address.len() > 255
        || address.chars().any(char::is_control)
        || address.chars().any(char::is_whitespace)
    {
        return Err("服务器地址无效".into());
    }
    if let Ok(socket) = address.parse::<SocketAddr>() {
        return Ok((socket.ip().to_string(), socket.port()));
    }
    if let Some(rest) = address.strip_prefix('[') {
        let end = rest.find(']').ok_or("IPv6 地址格式无效")?;
        let host = &rest[..end];
        let suffix = &rest[end + 1..];
        let port = if suffix.is_empty() {
            25565
        } else {
            suffix
                .strip_prefix(':')
                .ok_or("IPv6 地址格式无效")?
                .parse::<u16>()
                .map_err(|_| "服务器端口无效")?
        };
        if host.is_empty() || port == 0 {
            return Err("服务器地址无效".into());
        }
        return Ok((host.to_string(), port));
    }
    if let Some((host, raw_port)) = address.rsplit_once(':') {
        if !host.contains(':') {
            let port = raw_port.parse::<u16>().map_err(|_| "服务器端口无效")?;
            if host.is_empty() || port == 0 {
                return Err("服务器地址无效".into());
            }
            return Ok((host.to_string(), port));
        }
    }
    if address.contains(':') {
        return Err("IPv6 地址请使用 [地址] 或 [地址]:端口 格式".into());
    }
    Ok((address.to_string(), 25565))
}

async fn send_handshake(stream: &mut TcpStream, host: &str, port: u16) -> Result<(), String> {
    let mut packet = vec![0];
    write_varint(&mut packet, -1);
    write_varint(&mut packet, host.len() as i32);
    packet.extend_from_slice(host.as_bytes());
    packet.extend_from_slice(&port.to_be_bytes());
    write_varint(&mut packet, 1);
    let mut framed = Vec::with_capacity(packet.len() + 5);
    write_varint(&mut framed, packet.len() as i32);
    framed.extend_from_slice(&packet);
    stream
        .write_all(&framed)
        .await
        .map_err(|e| format!("发送服务器握手失败: {e}"))
}

async fn send_ping(stream: &mut TcpStream) -> Result<(), String> {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;
    let mut packet = vec![1];
    packet.extend_from_slice(&timestamp.to_be_bytes());
    let mut framed = vec![9];
    framed.extend_from_slice(&packet);
    stream
        .write_all(&framed)
        .await
        .map_err(|e| format!("发送延迟探测失败: {e}"))
}

async fn read_pong(stream: &mut TcpStream, started: Instant) -> Result<u128, String> {
    let packet = read_packet(stream).await?;
    let mut offset = 0;
    if read_varint_slice(&packet, &mut offset)? != 1 || packet.len().saturating_sub(offset) != 8 {
        return Err("服务器延迟响应无效".into());
    }
    Ok(started.elapsed().as_millis())
}

async fn read_packet(stream: &mut TcpStream) -> Result<Vec<u8>, String> {
    let length = read_varint(stream)
        .await
        .map_err(|e| format!("读取服务器响应长度失败: {e}"))?;
    let length = usize::try_from(length).map_err(|_| "服务器响应长度无效")?;
    if length == 0 || length > MAX_PACKET {
        return Err("服务器响应长度超出限制".into());
    }
    let mut packet = vec![0; length];
    stream
        .read_exact(&mut packet)
        .await
        .map_err(|e| format!("读取服务器响应失败: {e}"))?;
    Ok(packet)
}

fn parse_status_packet(packet: &[u8]) -> Result<Value, String> {
    let mut offset = 0;
    if read_varint_slice(packet, &mut offset)? != 0 {
        return Err("服务器状态包类型无效".into());
    }
    let length = usize::try_from(read_varint_slice(packet, &mut offset)?)
        .map_err(|_| "服务器状态数据长度无效")?;
    if length > MAX_PACKET || packet.len().saturating_sub(offset) != length {
        return Err("服务器状态数据长度无效".into());
    }
    serde_json::from_slice(&packet[offset..]).map_err(|e| format!("服务器状态 JSON 无效: {e}"))
}

fn component_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(items) => items.iter().map(component_text).collect::<String>(),
        Value::Object(object) => {
            let mut text = object
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if let Some(extra) = object.get("extra").and_then(Value::as_array) {
                text.push_str(&extra.iter().map(component_text).collect::<String>());
            }
            text
        }
        _ => String::new(),
    }
}

async fn read_varint<R: tokio::io::AsyncRead + Unpin>(reader: &mut R) -> io::Result<i32> {
    let mut value = 0i32;
    for shift in (0..35).step_by(7) {
        let mut byte = [0];
        reader.read_exact(&mut byte).await?;
        value |= i32::from(byte[0] & 0x7f).wrapping_shl(shift);
        if byte[0] & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        "VarInt too long",
    ))
}

fn read_varint_slice(bytes: &[u8], offset: &mut usize) -> Result<i32, String> {
    let mut value = 0i32;
    for shift in (0..35).step_by(7) {
        let byte = *bytes.get(*offset).ok_or("服务器响应意外结束")?;
        *offset += 1;
        value |= i32::from(byte & 0x7f).wrapping_shl(shift);
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err("服务器响应 VarInt 无效".into())
}

fn write_varint(out: &mut Vec<u8>, mut value: i32) {
    loop {
        if value & !0x7f == 0 {
            out.push(value as u8);
            return;
        }
        out.push(((value & 0x7f) | 0x80) as u8);
        value = ((value as u32) >> 7) as i32;
    }
}
