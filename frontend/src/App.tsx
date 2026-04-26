import { useState } from 'react'
import { Sidebar } from './components/layout/Sidebar'
import { HomePage } from './components/pages/HomePage'
import { LaunchPage } from './components/pages/LaunchPage'
import { SettingsPage } from './components/pages/SettingsPage'
import { NewsPage } from './components/pages/NewsPage'
import { AppProvider } from './contexts/AppProvider'

function AppContent() {
  const [activePage, setActivePage] = useState('home')

  const renderPage = () => {
    switch (activePage) {
      case 'home':
        return <HomePage />
      case 'launch':
        return <LaunchPage />
      case 'settings':
        return <SettingsPage />
      case 'news':
        return <NewsPage />
      default:
        return <HomePage />
    }
  }

  return (
    <div style={{
      display: 'flex',
      minHeight: '100vh',
      background: 'var(--bg-deep)',
    }}>
      <Sidebar activePage={activePage} onNavigate={setActivePage} />
      <main style={{
        marginLeft: '64px',
        flex: 1,
        minHeight: '100vh',
      }}>
        {renderPage()}
      </main>
    </div>
  )
}

export default function App() {
  return (
    <AppProvider>
      <AppContent />
    </AppProvider>
  )
}