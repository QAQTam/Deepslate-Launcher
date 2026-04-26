import { createContext, useContext, useState, useEffect, type ReactNode } from 'react'
import { listen } from '@tauri-apps/api/event'

interface GameState {
  state: 'launching' | 'running' | 'stopped'
  pid?: number
  exit_code?: number
}

interface GameContextType {
  launching: boolean
  gameRunning: boolean
  setLaunching: (v: boolean) => void
  setGameRunning: (v: boolean) => void
}

const GameContext = createContext<GameContextType | null>(null)

export function GameProvider({ children }: { children: ReactNode }) {
  const [gameRunning, setGameRunning] = useState(false)
  const [launching, setLaunching] = useState(false)

  useEffect(() => {
    const unlisten = listen<GameState>('game-state', (payload) => {
      if (payload.state === 'launching') {
        setLaunching(true)
        setGameRunning(true)
      } else if (payload.state === 'running') {
        setLaunching(false)
        setGameRunning(true)
      } else if (payload.state === 'stopped') {
        setGameRunning(false)
        setLaunching(false)
      }
    })
    return () => { unlisten.then(fn => fn()) }
  }, [])

  return (
    <GameContext.Provider value={{ launching, gameRunning, setLaunching, setGameRunning }}>
      {children}
    </GameContext.Provider>
  )
}

export function useGame() {
  const ctx = useContext(GameContext)
  if (!ctx) throw new Error('useGame must be used within GameProvider')
  return ctx
}