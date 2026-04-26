import { motion } from 'framer-motion'

interface PageContainerProps {
  children: React.ReactNode
  title: string
}

const pageVariants = {
  initial: { opacity: 0, y: 20 },
  animate: { opacity: 1, y: 0 },
  exit: { opacity: 0, y: -20 },
}

const staggerContainer = {
  animate: {
    transition: {
      staggerChildren: 0.1,
    },
  },
}

const staggerItem = {
  initial: { opacity: 0, y: 20 },
  animate: { opacity: 1, y: 0 },
}

export function PageContainer({ children, title }: PageContainerProps) {
  return (
    <motion.div
      initial="initial"
      animate="animate"
      exit="exit"
      variants={pageVariants}
      transition={{ duration: 0.3 }}
      style={{ padding: '32px' }}
    >
      <motion.h2
        variants={staggerItem}
        style={{
          fontFamily: 'var(--font-display)',
          fontSize: '1.5rem',
          fontWeight: 700,
          marginBottom: '24px',
          color: 'var(--text-primary)',
        }}
      >
        {title}
      </motion.h2>
      <motion.div variants={staggerContainer}>
        {children}
      </motion.div>
    </motion.div>
  )
}