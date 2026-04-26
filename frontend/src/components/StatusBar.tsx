import { useAppState } from '../hooks/useAppState'

export function StatusBar() {
  const { status } = useAppState()

  return (
    <section className="section status-section">
      <p className="status">
        <span className="status-dot" />
        {status}
      </p>
    </section>
  )
}
