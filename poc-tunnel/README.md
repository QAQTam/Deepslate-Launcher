# Deepslate P2P Tunnel 原型验证工具 (PoC)

这是一个完全独立的轻量级网络打洞与 TCP 代理验证原型，专为验证**无公网环境（如云电脑、严格 NAT、校园网）下的 Minecraft P2P 联机可行性**而设计。

底层基于 **Iroh 1.4 (QUIC + STUN + UPnP + 公网 Relay 保底)**，并以 **Ed25519 公钥** 作为节点唯一标识，与 Deepslate Launcher 现有的 DL2 好友身份体系完全吻合。

---

## 快速实测流程（以云电脑与本机为例）

编译产物位置：`poc-tunnel/target/release/poc-tunnel`（已编译就绪）

### 场景一：快速验证打洞连通性（无需启动 Minecraft）

#### 1. 云电脑端（作为房主 Host）运行：
```bash
./poc-tunnel host --echo
```
- 程序会自动初始化 Endpoint、探测 NAT、上线 Relay，并生成一段联机票据：
```text
==================================================
     Deepslate P2P Tunnel - 房主 (Host) 模式
==================================================
[内置测试] 已启动内置测试服务，监听 127.0.0.1:44645
[网络] 正在初始化 Iroh P2P Endpoint，探测 STUN 与中继...
[身份] 本节点 Ed25519 NodeId: <公钥>
[网络] 已连通 Home Relay 服务器，打洞与中继就绪！

--------------------------------------------------
房间已开启！请复制下方联机票据发送给客机：

  DSTUN-eyJ0aXRsZSI6...

--------------------------------------------------
```

#### 2. 本机端（作为客机 Client）进行连通性诊断：
```bash
./poc-tunnel ping --ticket "DSTUN-eyJ0aXRsZSI6..."
```
- 会立即向云电脑发起 UDP 打洞。输出中会清晰显示：
  - 是否连通
  - 穿透路径：`【P2P 直连 UDP 打洞成功】` 还是 `【保底中继 Relay】`
  - 往返 RTT 延迟与应用层响应。

#### 3. 本机端开启透明代理体验：
```bash
./poc-tunnel client --ticket "DSTUN-eyJ0aXRsZSI6..." --bind-port 39999
```
此时本机终端执行：
```bash
curl http://127.0.0.1:39999
```
将直接收到云电脑内置服务返回的穿透欢迎信息！

---

### 场景二：真实 Minecraft 26.3 游戏联机体验

#### 1. 云电脑端：
启动 Minecraft 26.3 并进入单人世界，点击“公开为局域网”（或运行 Dedicated Server，默认端口 25565）。
运行：
```bash
./poc-tunnel host --target-port 25565
```

#### 2. 本机端：
运行代理并绑定本地 25565：
```bash
./poc-tunnel client --ticket "<云电脑生成的票据>" --bind-port 25565
```

#### 3. 进入游戏：
本机的 Minecraft 打开多人游戏，直接连接 `127.0.0.1:25565`（或者使用 Bridge 的 `JoinServer`），即可跨越无公网 NAT 障碍直接连入云电脑的世界！
