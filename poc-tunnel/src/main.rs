use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use clap::{Parser, Subcommand};
use iroh::{
    endpoint::{presets, Connection, RecvStream, SendStream},
    Endpoint, EndpointAddr,
};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const ALPN: &[u8] = b"deepslate-tunnel/v1";
const TICKET_PREFIX: &str = "DSTUN-";

#[derive(Parser, Debug)]
#[command(name = "poc-tunnel", about = "Deepslate P2P Tunnel Proof of Concept")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// 运行房主端（Host）：暴露本地端口或内置测试服务给对端
    Host {
        /// 本地需要代理的目标端口（如 Minecraft LAN 端口或 25565）
        #[arg(short, long, default_value_t = 25565)]
        target_port: u16,

        /// 是否启动内置测试 Echo 服务（无需启动 Minecraft 即可验证打洞）
        #[arg(long)]
        echo: bool,

        /// 房间标题/备注
        #[arg(long, default_value = "Deepslate-Test-Room")]
        title: String,
    },

    /// 运行客机端（Client）：在本地暴露端口，连接将透明隧道转发到房主
    Client {
        /// 房主生成的联机票据 (DSTUN-...)
        #[arg(short, long)]
        ticket: String,

        /// 客机本地监听的端口（Minecraft 客户端直连 127.0.0.1:<bind-port>）
        #[arg(short, long, default_value_t = 25565)]
        bind_port: u16,
    },

    /// 一键网络诊断：测试与房主的打洞、中继状态与往返延迟
    Ping {
        /// 房主生成的联机票据 (DSTUN-...)
        #[arg(short, long)]
        ticket: String,
    },
}

/// 房间联机票据
#[derive(Serialize, Deserialize, Debug, Clone)]
struct RoomTicket {
    title: String,
    target_port: u16,
    addr: EndpointAddr,
}

impl RoomTicket {
    fn encode(&self) -> Result<String> {
        let json = serde_json::to_vec(self)?;
        let b64 = URL_SAFE_NO_PAD.encode(json);
        Ok(format!("{}{}", TICKET_PREFIX, b64))
    }

    fn decode(raw: &str) -> Result<Self> {
        let s = raw.trim();
        let payload = if let Some(stripped) = s.strip_prefix(TICKET_PREFIX) {
            stripped
        } else {
            s
        };
        let bytes = URL_SAFE_NO_PAD
            .decode(payload)
            .context("票据 Base64URL 解码失败")?;
        let ticket: RoomTicket =
            serde_json::from_slice(&bytes).context("票据 JSON 反序列化失败")?;
        Ok(ticket)
    }
}

/// 格式化连接路径状态（直连还是 Relay 保底）
fn format_path_info(conn: &Connection) -> String {
    let paths = conn.paths();
    for p in paths.iter() {
        if p.is_selected() {
            let kind = if p.is_relay() {
                "【保底中继 Relay】"
            } else if p.is_ip() {
                "【P2P 直连 UDP 打洞成功】"
            } else {
                "【通道中】"
            };
            return format!("{} 远端: {}, RTT: {:?}", kind, p.remote_addr(), p.rtt());
        }
    }
    format!("【初始握手状态】 路径数: {}", paths.len())
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Host {
            target_port,
            echo,
            title,
        } => run_host(target_port, echo, title).await,
        Commands::Client { ticket, bind_port } => run_client(ticket, bind_port).await,
        Commands::Ping { ticket } => run_ping(ticket).await,
    }
}

/// 房主逻辑
async fn run_host(mut target_port: u16, echo: bool, title: String) -> Result<()> {
    println!("==================================================");
    println!("     Deepslate P2P Tunnel - 房主 (Host) 模式");
    println!("==================================================");

    if echo {
        let echo_port = start_builtin_echo_server().await?;
        println!("[内置测试] 已启动内置测试服务，监听 127.0.0.1:{}", echo_port);
        target_port = echo_port;
    }

    println!("[网络] 正在初始化 Iroh P2P Endpoint，探测 STUN 与中继...");
    let endpoint = Endpoint::builder(presets::N0)
        .alpns(vec![ALPN.to_vec()])
        .bind()
        .await
        .context("绑定 P2P Endpoint 失败")?;

    let node_id = endpoint.id();
    println!("[身份] 本节点 Ed25519 NodeId: {}", node_id);
    println!("[网络] 正在连向公共保底中继以确保无公网穿透能力...");

    // 等待连接到 Relay（最多等待 15 秒）
    match tokio::time::timeout(Duration::from_secs(15), endpoint.online()).await {
        Ok(_) => println!("[网络] 已连通 Home Relay 服务器，打洞与中继就绪！"),
        Err(_) => println!("[警告] 连接 Relay 超时，将仅尝试局域网/直连。"),
    }

    let addr = endpoint.addr();
    let ticket = RoomTicket {
        title: title.clone(),
        target_port,
        addr,
    };
    let ticket_str = ticket.encode()?;

    println!("\n--------------------------------------------------");
    println!("房间已开启！请复制下方联机票据发送给客机：");
    println!("\n  {}\n", ticket_str);
    println!("--------------------------------------------------");
    println!("[配置] 目标映射端口: 127.0.0.1:{}", target_port);
    println!("[状态] 正在等待客机连入... (按 Ctrl+C 退出)\n");

    let counter = Arc::new(AtomicU64::new(1));
    while let Some(incoming) = endpoint.accept().await {
        let counter = counter.clone();
        tokio::spawn(async move {
            let conn_num = counter.fetch_add(1, Ordering::SeqCst);
            match incoming.await {
                Ok(conn) => {
                    let remote_id = conn.remote_id().to_string();
                    let path_info = format_path_info(&conn);
                    println!(
                        "[连接 #{}] 客机接入！NodeId: {}\n             状态: {}",
                        conn_num, remote_id, path_info
                    );
                    handle_host_connection(conn, target_port, conn_num).await;
                }
                Err(e) => {
                    eprintln!("[错误] 接入握手失败: {}", e);
                }
            }
        });
    }

    Ok(())
}

async fn handle_host_connection(conn: Connection, target_port: u16, conn_num: u64) {
    loop {
        match conn.accept_bi().await {
            Ok((send, recv)) => {
                let target_addr: SocketAddr = format!("127.0.0.1:{}", target_port)
                    .parse()
                    .unwrap();
                println!("[流 #{}] 收到新数据流，正在桥接本地服务 {}", conn_num, target_addr);

                tokio::spawn(async move {
                    match TcpStream::connect(target_addr).await {
                        Ok(tcp) => {
                            let _ = bridge_stream(tcp, send, recv).await;
                        }
                        Err(e) => {
                            eprintln!("[错误] 无法连接到本地目标端口 {}: {}", target_addr, e);
                        }
                    }
                });
            }
            Err(e) => {
                println!("[连接 #{}] 连接已断开: {}", conn_num, e);
                break;
            }
        }
    }
}

/// 客机逻辑
async fn run_client(ticket_str: String, bind_port: u16) -> Result<()> {
    let ticket = RoomTicket::decode(&ticket_str)?;

    println!("==================================================");
    println!("     Deepslate P2P Tunnel - 客机 (Client) 模式");
    println!("==================================================");
    println!("[房间] 标题: {}", ticket.title);
    println!("[房间] 房主 NodeId: {}", ticket.addr.id);
    println!(
        "[房间] 房主直连候选: {:?}",
        ticket.addr.ip_addrs().collect::<Vec<_>>()
    );

    println!("[网络] 正在初始化本地 Iroh Endpoint...");
    let endpoint = Endpoint::builder(presets::N0)
        .alpns(vec![ALPN.to_vec()])
        .bind()
        .await
        .context("绑定 P2P Endpoint 失败")?;

    // 尝试上线保底中继
    let _ = tokio::time::timeout(Duration::from_secs(8), endpoint.online()).await;

    let bind_addr: SocketAddr = format!("127.0.0.1:{}", bind_port).parse()?;
    let listener = TcpListener::bind(bind_addr)
        .await
        .with_context(|| format!("客机无法绑定本地端口 {}", bind_addr))?;

    println!("\n[代理就绪] 本地代理已监听: {}", bind_addr);
    println!(">>> 请启动 Minecraft 并连接服务器: 127.0.0.1:{} <<<", bind_port);
    println!(">>> 或者在终端执行: curl http://127.0.0.1:{} <<<\n", bind_port);

    let endpoint = Arc::new(endpoint);
    let target_addr = ticket.addr;
    let stream_seq = Arc::new(AtomicU64::new(1));

    while let Ok((tcp, peer)) = listener.accept().await {
        let endpoint = endpoint.clone();
        let target_addr = target_addr.clone();
        let seq = stream_seq.fetch_add(1, Ordering::SeqCst);

        tokio::spawn(async move {
            println!(
                "[客户端 #{}] 检测到本地连接 ({})，正在打洞连接房主...",
                seq, peer
            );
            let start = Instant::now();

            match endpoint.connect(target_addr, ALPN).await {
                Ok(conn) => {
                    let handshake_time = start.elapsed();
                    let path_info = format_path_info(&conn);
                    println!(
                        "[客户端 #{}] 打洞完成！耗时: {:?}\n              状态: {}",
                        seq, handshake_time, path_info
                    );

                    match conn.open_bi().await {
                        Ok((send, recv)) => {
                            let _ = bridge_stream(tcp, send, recv).await;
                        }
                        Err(e) => {
                            eprintln!("[客户端 #{}] 打开 P2P 双向流失败: {}", seq, e);
                        }
                    }
                }
                Err(e) => {
                    eprintln!("[客户端 #{}] 连接房主失败: {}", seq, e);
                }
            }
        });
    }

    Ok(())
}

/// 一键 Ping 诊断逻辑
async fn run_ping(ticket_str: String) -> Result<()> {
    let ticket = RoomTicket::decode(&ticket_str)?;

    println!("==================================================");
    println!("     Deepslate P2P Tunnel - 连通性与打洞诊断");
    println!("==================================================");
    println!("[目标] 房主 NodeId: {}", ticket.addr.id);
    println!("[目标] 房间名: {}", ticket.title);

    let endpoint = Endpoint::builder(presets::N0)
        .alpns(vec![ALPN.to_vec()])
        .bind()
        .await?;

    println!("[诊断] 正在探测网络并向房主发起打洞连接...");
    let start = Instant::now();

    match tokio::time::timeout(Duration::from_secs(15), endpoint.connect(ticket.addr, ALPN)).await {
        Ok(Ok(conn)) => {
            let elapsed = start.elapsed();
            let path_info = format_path_info(&conn);
            println!("\n✅ 穿透成功！连通性诊断结果：");
            println!("   - 建立连接总耗时: {:?}", elapsed);
            println!("   - 连接路径详情: {}", path_info);

            // 尝试打开流发送一个短包验证应用层
            match conn.open_bi().await {
                Ok((mut send, mut recv)) => {
                    let ping_msg = b"PING\n";
                    send.write_all(ping_msg).await?;
                    send.finish()?;

                    let mut resp = [0u8; 128];
                    let read_res = tokio::time::timeout(Duration::from_secs(3), recv.read(&mut resp)).await;
                    match read_res {
                        Ok(Ok(Some(n))) => {
                            let text = String::from_utf8_lossy(&resp[..n]);
                            println!("   - 应用层端到端响应: {}", text.trim());
                        }
                        _ => {
                            println!("   - 传输层连通完成（目标端口未返回文本响应或为 Minecraft 二进制握手）。");
                        }
                    }
                }
                Err(e) => {
                    println!("   - 打开测试流失败: {}", e);
                }
            }
        }
        Ok(Err(e)) => {
            bail!("❌ 打洞连接失败: {}", e);
        }
        Err(_) => {
            bail!("❌ 连接超时（15秒）。请检查云电脑网络或安全组是否拦截了全部出站 UDP。");
        }
    }

    Ok(())
}

/// 双向无阻碍数据桥接（单侧断开即整体对齐释放）
async fn bridge_stream(tcp: TcpStream, mut quic_send: SendStream, mut quic_recv: RecvStream) -> Result<()> {
    let (mut tcp_read, mut tcp_write) = tcp.into_split();

    let quic_to_tcp = async {
        println!("[Bridge] Starting quic_to_tcp copy...");
        match tokio::io::copy(&mut quic_recv, &mut tcp_write).await {
            Ok(bytes) => println!("[Bridge] quic_to_tcp copied {} bytes", bytes),
            Err(e) => println!("[Bridge] quic_to_tcp error: {}", e),
        }
        let _ = tcp_write.shutdown().await;
        println!("[Bridge] tcp_write shutdown completed");
    };

    let tcp_to_quic = async {
        println!("[Bridge] Starting tcp_to_quic copy...");
        match tokio::io::copy(&mut tcp_read, &mut quic_send).await {
            Ok(bytes) => println!("[Bridge] tcp_to_quic copied {} bytes", bytes),
            Err(e) => println!("[Bridge] tcp_to_quic error: {}", e),
        }
        let _ = quic_send.finish();
        println!("[Bridge] quic_send finished");
    };

    tokio::join!(quic_to_tcp, tcp_to_quic);

    Ok(())
}

/// 启动内置的轻量测试 Echo / HTTP 服务
async fn start_builtin_echo_server() -> Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();

    tokio::spawn(async move {
        while let Ok((mut socket, peer)) = listener.accept().await {
            println!("[EchoServer] Accepted connection from {}", peer);
            tokio::spawn(async move {
                let mut buf = [0u8; 1024];
                match socket.read(&mut buf).await {
                    Ok(n) if n > 0 => {
                        let request_str = String::from_utf8_lossy(&buf[..n]);
                        println!("[EchoServer] Read {} bytes: {}", n, request_str.trim());
                        let response = if request_str.starts_with("GET ") {
                            format!(
                                "HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: 68\r\nConnection: close\r\n\r\n🎉 恭喜！来自 Deepslate P2P 隧道的连接已穿透成功！(HTTP Echo)\r\n"
                            )
                        } else {
                            format!(
                                "PONG: 收到数据 ({} bytes): {}\n",
                                n,
                                request_str.trim()
                            )
                        };
                        let _ = socket.write_all(response.as_bytes()).await;
                        let _ = socket.shutdown().await;
                        println!("[EchoServer] Response sent and socket shutdown");
                    }
                    Ok(_) => {
                        println!("[EchoServer] Read 0 bytes (EOF)");
                    }
                    Err(e) => {
                        println!("[EchoServer] Read error: {}", e);
                    }
                }
            });
        }
    });

    Ok(port)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_p2p_loopback() -> Result<()> {
        let echo_port = start_builtin_echo_server().await?;

        let server_ep = Endpoint::builder(presets::Minimal)
            .alpns(vec![ALPN.to_vec()])
            .bind()
            .await?;
        let mut server_addr = server_ep.addr();
        for sock in server_ep.bound_sockets() {
            if sock.is_ipv4() {
                server_addr = server_addr.with_ip_addr(SocketAddr::new(
                    std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
                    sock.port(),
                ));
            }
        }
        println!("Server EndpointAddr with loopback: {:?}", server_addr);

        let s_handle = tokio::spawn(async move {
            if let Some(incoming) = server_ep.accept().await {
                println!("Server got incoming connection, awaiting handshake...");
                let conn = incoming.await.unwrap();
                println!("Server handshake done, accepting bi stream...");
                let (send, recv) = conn.accept_bi().await.unwrap();
                println!("Server stream accepted, connecting to echo server...");
                let tcp = TcpStream::connect(format!("127.0.0.1:{}", echo_port)).await.unwrap();
                bridge_stream(tcp, send, recv).await.unwrap();
                println!("Server bridge_stream finished, waiting for client to close...");
                conn.closed().await;
                println!("Server connection closed cleanly");
            }
        });

        let client_ep = Endpoint::builder(presets::Minimal)
            .alpns(vec![ALPN.to_vec()])
            .bind()
            .await?;

        println!("[Client] Connecting to server...");
        let conn = client_ep.connect(server_addr, ALPN).await?;
        println!("[Client] Connected, opening bi stream...");
        let (mut send, mut recv) = conn.open_bi().await?;
        println!("[Client] Opened bi stream, sending data...");
        send.write_all(b"Hello P2P\n").await?;
        send.finish()?;
        println!("[Client] Data sent, awaiting recv.read...");

        let resp_bytes = recv.read_to_end(1024).await?;
        let resp = String::from_utf8_lossy(&resp_bytes);
        println!("[Client] Read {} bytes: {}", resp_bytes.len(), resp);
        assert!(resp.contains("PONG"));

        drop(send);
        drop(recv);
        conn.close(0u8.into(), b"done");

        let _ = s_handle.await;
        Ok(())
    }
}
