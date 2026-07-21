import { useEffect, useState } from "react"
import { invoke } from "@tauri-apps/api/core"
import type { ModInfo } from "../../types"
import { PageContainer } from "../layout/PageContainer"

export function ModsPage() {
  const [mods, setMods] = useState<ModInfo[]>([])
  const [loading, setLoading] = useState(true)

  useEffect(() => {
    loadMods()
  }, [])

  async function loadMods() {
    try {
      setLoading(true)
      const cfg: any = await invoke("get_config")
      const result: ModInfo[] = await invoke("list_mods", { gameDir: cfg.game_dir })
      setMods(result)
    } catch (e) {
      console.error("Failed to load mods:", e)
    } finally {
      setLoading(false)
    }
  }

  async function handleToggle(mod: ModInfo) {
    try {
      const cfg: any = await invoke("get_config")
      await invoke("toggle_mod", {
        gameDir: cfg.game_dir,
        modId: mod.id,
        enabled: !mod.enabled,
      })
      await loadMods()
    } catch (e) {
      console.error("Failed to toggle mod:", e)
    }
  }

  async function handleInstallLoader() {
    try {
      const cfg: any = await invoke("get_config")
      const result = await invoke("install_ghastloader", { gameDir: cfg.game_dir })
      alert(`GhastLoader: ${result}`)
    } catch (e: any) {
      alert(`安装失败: ${e}`)
    }
  }

  return (
    <PageContainer title="Mods — GhastLoader">
      <div style={{ padding: "1.5rem", maxWidth: 700 }}>
        {/* 安装 GhastLoader */}
        <div style={cardStyle}>
          <div style={cardHeaderStyle}>
            <h3 style={{ margin: 0 }}>GhastLoader Agent</h3>
            <button onClick={handleInstallLoader} style={actionBtnStyle}>
              安装 / 更新
            </button>
          </div>
          <p style={{ color: "#888", fontSize: "0.85rem", margin: "0.25rem 0 0" }}>
            Java Agent 事件总线式 mod 加载器。需要在启动前安装一次。
          </p>
        </div>

        {/* Mod 列表 */}
        <h2 style={{ marginTop: "2rem", marginBottom: "1rem" }}>已安装的 Mod</h2>
        {loading ? (
          <p style={{ color: "#888" }}>加载中...</p>
        ) : mods.length === 0 ? (
          <div style={emptyStateStyle}>
            <p style={{ color: "#888", margin: 0 }}>
              还没有安装任何 mod。
            </p>
            <p style={{ color: "#666", fontSize: "0.8rem", margin: "0.25rem 0 0" }}>
              将 .jar 文件放入 game 目录下的 mods/ 文件夹即可。
            </p>
          </div>
        ) : (
          <div style={{ display: "flex", flexDirection: "column", gap: "0.75rem" }}>
            {mods.map((mod) => (
              <div
                key={mod.id}
                style={{
                  ...cardStyle,
                  opacity: mod.enabled ? 1 : 0.5,
                }}
              >
                <div style={cardHeaderStyle}>
                  <div>
                    <strong>{mod.id}</strong>
                    <span style={{ color: "#888", marginLeft: "0.75rem", fontSize: "0.85rem" }}>
                      v{mod.version}
                    </span>
                  </div>
                  <button
                    onClick={() => handleToggle(mod)}
                    style={{
                      ...actionBtnStyle,
                      background: mod.enabled ? "#4caf50" : "#666",
                    }}
                  >
                    {mod.enabled ? "已启用" : "已禁用"}
                  </button>
                </div>
                <p
                  style={{
                    color: "#888",
                    fontSize: "0.8rem",
                    margin: "0.25rem 0 0",
                    fontFamily: "monospace",
                  }}
                >
                  {mod.entry}
                </p>
              </div>
            ))}
          </div>
        )}
      </div>
    </PageContainer>
  )
}

const cardStyle: React.CSSProperties = {
  background: "#1a1a2e",
  border: "1px solid #2a2a4a",
  borderRadius: 8,
  padding: "1rem",
}

const cardHeaderStyle: React.CSSProperties = {
  display: "flex",
  justifyContent: "space-between",
  alignItems: "center",
}

const actionBtnStyle: React.CSSProperties = {
  background: "#6366f1",
  color: "#fff",
  border: "none",
  borderRadius: 6,
  padding: "0.4rem 1rem",
  cursor: "pointer",
  fontSize: "0.85rem",
}

const emptyStateStyle: React.CSSProperties = {
  background: "#1a1a2e",
  border: "2px dashed #2a2a4a",
  borderRadius: 8,
  padding: "2rem",
  textAlign: "center",
}
