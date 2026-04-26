import { useState, useEffect } from 'react'
import { motion } from 'framer-motion'
import { PageContainer } from '../layout/PageContainer'
import { fetchMinecraftNews, type NewsItem } from '../../services/newsApi'

const CATEGORIES = [
  { id: 'all', label: '全部' },
  { id: 'official', label: '官方' },
  { id: 'community', label: '社区' },
  { id: 'mod', label: 'Mod' },
] as const

export function NewsPage() {
  const [activeCategory, setActiveCategory] = useState<typeof CATEGORIES[number]['id']>('all')
  const [news, setNews] = useState<NewsItem[]>([])
  const [loading, setLoading] = useState(true)

  useEffect(() => {
    async function loadNews() {
      const items = await fetchMinecraftNews()
      setNews(items)
      setLoading(false)
    }
    loadNews()
  }, [])

  const filteredNews = activeCategory === 'all'
    ? news
    : news.filter((item) => item.category === activeCategory)

  return (
    <PageContainer title="新闻动态">
      {/* Category Filter */}
      <motion.div
        variants={{ initial: { opacity: 0, y: 20 }, animate: { opacity: 1, y: 0 } }}
        style={{
          display: 'flex',
          gap: '8px',
          marginBottom: '24px',
          flexWrap: 'wrap',
        }}
      >
        {CATEGORIES.map((cat) => (
          <button
            key={cat.id}
            onClick={() => setActiveCategory(cat.id)}
            style={{
              padding: '8px 16px',
              borderRadius: 'var(--radius-full)',
              border: 'none',
              background: activeCategory === cat.id
                ? 'var(--accent-primary)'
                : 'rgba(139, 92, 246, 0.1)',
              color: activeCategory === cat.id
                ? 'white'
                : 'var(--text-secondary)',
              fontWeight: 500,
              cursor: 'pointer',
              transition: 'all 0.2s ease',
            }}
          >
            {cat.label}
          </button>
        ))}
      </motion.div>

      {/* News Grid */}
      {loading ? (
        <div style={{ textAlign: 'center', padding: '48px', color: 'var(--text-muted)' }}>
          加载中...
        </div>
      ) : (
        <motion.div
          variants={{ animate: { transition: { staggerChildren: 0.1 } } }}
          style={{
            display: 'grid',
            gridTemplateColumns: 'repeat(auto-fill, minmax(300px, 1fr))',
            gap: '20px',
          }}
        >
          {filteredNews.map((item) => (
            <NewsCard key={item.id} item={item} />
          ))}
        </motion.div>
      )}
    </PageContainer>
  )
}

function NewsCard({ item }: { item: NewsItem }) {
  return (
    <motion.a
      href={item.link}
      target="_blank"
      rel="noopener noreferrer"
      variants={{
        initial: { opacity: 0, y: 20 },
        animate: { opacity: 1, y: 0 },
      }}
      whileHover={{ scale: 1.02, boxShadow: 'var(--glow-purple)' }}
      className="glass-card"
      style={{
        padding: '20px',
        display: 'block',
        textDecoration: 'none',
        color: 'inherit',
      }}
    >
      <span style={{
        fontSize: '0.7rem',
        color: 'var(--accent-primary)',
        textTransform: 'uppercase',
        letterSpacing: '0.05em',
      }}>
        {item.category}
      </span>
      <h4 style={{
        fontFamily: 'var(--font-display)',
        fontSize: '1rem',
        fontWeight: 600,
        color: 'var(--text-primary)',
        marginTop: '8px',
        marginBottom: '8px',
      }}>
        {item.title}
      </h4>
      <p style={{
        fontSize: '0.875rem',
        color: 'var(--text-secondary)',
        lineHeight: 1.5,
        display: '-webkit-box',
        WebkitLineClamp: 3,
        WebkitBoxOrient: 'vertical',
        overflow: 'hidden',
      }}>
        {item.description}
      </p>
      <p style={{
        fontSize: '0.75rem',
        color: 'var(--text-muted)',
        marginTop: '12px',
      }}>
        {new Date(item.pubDate).toLocaleDateString('zh-CN')}
      </p>
    </motion.a>
  )
}
