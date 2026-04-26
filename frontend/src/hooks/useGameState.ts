import { useState } from 'react'
import { useTauriEvent } from './useTauriEvent'

interface GameState {
  state: 'launching' | 'running' | 'stopped'
  pid?: number
  exit_code?: number
}

export function useGameState() {
  const [gameRunning, setGameRunning] = useState(false)
  const [launching, setLaunching] = useState(false)

  useTauriEvent<GameState>('game-state', (payload) => {
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

  return { gameRunning, setGameRunning, launching, setLaunching }
}
