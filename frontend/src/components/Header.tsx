export function Header() {
  return (
    <header className="header">
      <div className="header-brand">
        <span style={{
          fontFamily: 'var(--font-display)',
          fontWeight: 700,
          fontSize: '1.05rem',
          color: 'var(--text-primary)',
        }}>
          Obsidian Portal
        </span>
      </div>
      <span className="header-subtitle">Minecraft Launcher</span>
    </header>
  );
}
