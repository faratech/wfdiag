import React from 'react'
import {
  Card,
  Button,
  Title2,
  Title3,
  Body1,
  Caption1,
  makeStyles,
  tokens,
  shorthands,
  Divider,
  ProgressBar
} from '@fluentui/react-components'
import {
  Flash20Regular,
  Search20Regular,
  Play20Regular,
  History20Regular,
  Copy20Regular,
  Save20Regular,
  CheckmarkCircle20Regular,
  ErrorCircle20Regular,
  Clock20Regular
} from '@fluentui/react-icons'

const useStyles = makeStyles({
  container: {
    maxWidth: '1400px',
    ...shorthands.margin('0', 'auto'),
    ...shorthands.padding(tokens.spacingVerticalXL, tokens.spacingHorizontalXXL),
  },

  welcomeCard: {
    background: 'linear-gradient(135deg, rgba(59, 130, 246, 0.1), rgba(139, 92, 246, 0.1))',
    ...shorthands.border('1px', 'solid', 'rgba(59, 130, 246, 0.3)'),
    ...shorthands.padding(tokens.spacingVerticalXXL),
    textAlign: 'center',
    position: 'relative',
    ...shorthands.overflow('hidden'),

    '&::before': {
      content: '""',
      position: 'absolute',
      top: 0,
      left: 0,
      right: 0,
      height: '4px',
      background: 'linear-gradient(90deg, #3B82F6, #8B5CF6, #3B82F6)',
      animation: 'shimmer 3s infinite',
    }
  },

  iconContainer: {
    width: '80px',
    height: '80px',
    ...shorthands.margin('0', 'auto', tokens.spacingVerticalL),
    ...shorthands.borderRadius('50%'),
    background: 'linear-gradient(135deg, #3B82F6, #8B5CF6)',
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'center',
    boxShadow: '0 8px 24px rgba(59, 130, 246, 0.4)',
  },

  scanButtons: {
    display: 'flex',
    justifyContent: 'center',
    ...shorthands.gap(tokens.spacingHorizontalL),
    ...shorthands.margin(tokens.spacingVerticalXL, '0'),
  },

  scanButton: {
    minWidth: '200px',
    height: '48px',
    fontSize: tokens.fontSizeBase400,
  },

  featureGrid: {
    display: 'grid',
    gridTemplateColumns: 'repeat(auto-fit, minmax(300px, 1fr))',
    ...shorthands.gap(tokens.spacingHorizontalL),
    marginTop: tokens.spacingVerticalXXL,
  },

  featureCard: {
    ...shorthands.padding(tokens.spacingVerticalL),
    backgroundColor: 'rgba(30, 41, 59, 0.4)',
    ...shorthands.border('1px', 'solid', 'rgba(255, 255, 255, 0.05)'),
    ...shorthands.borderRadius(tokens.borderRadiusMedium),
    transition: 'all 0.3s ease',

    ':hover': {
      backgroundColor: 'rgba(30, 41, 59, 0.6)',
      transform: 'translateY(-2px)',
      boxShadow: '0 4px 12px rgba(0, 0, 0, 0.2)',
    }
  },

  featureIcon: {
    fontSize: '24px',
    marginBottom: tokens.spacingVerticalS,
  },

  progressCard: {
    ...shorthands.padding(tokens.spacingVerticalXL),
    textAlign: 'center',
  },

  progressInfo: {
    display: 'flex',
    justifyContent: 'space-between',
    marginTop: tokens.spacingVerticalS,
  },

  resultsHeader: {
    display: 'flex',
    justifyContent: 'space-between',
    alignItems: 'center',
    marginBottom: tokens.spacingVerticalL,
  },

  resultsActions: {
    display: 'flex',
    ...shorthands.gap(tokens.spacingHorizontalS),
  },

  statsGrid: {
    display: 'grid',
    gridTemplateColumns: 'repeat(4, 1fr)',
    ...shorthands.gap(tokens.spacingHorizontalM),
    marginBottom: tokens.spacingVerticalL,
  },

  statCard: {
    ...shorthands.padding(tokens.spacingVerticalM),
    backgroundColor: 'rgba(30, 41, 59, 0.3)',
    ...shorthands.borderRadius(tokens.borderRadiusMedium),
    textAlign: 'center',
  }
})

export interface QuickActionPanelProps {
  onQuickScan: () => void
  onFullScan: () => void
  onCompare?: () => void
  onExport?: () => void
  onCopyToClipboard?: () => void
  isScanning?: boolean
  scanProgress?: number
  currentTask?: string
  hasResults?: boolean
  stats?: {
    totalTasks: number
    successfulTasks: number
    failedTasks: number
    duration: number
  }
}

export const QuickActionPanel: React.FC<QuickActionPanelProps> = ({
  onQuickScan,
  onFullScan,
  onCompare,
  onExport,
  onCopyToClipboard,
  isScanning = false,
  scanProgress = 0,
  currentTask = '',
  hasResults = false,
  stats
}) => {
  const styles = useStyles()

  if (isScanning) {
    return (
      <div className={styles.container}>
        <Card className={styles.progressCard}>
          <div className={styles.iconContainer} style={{ animation: 'pulse 2s infinite' }}>
            <Clock20Regular style={{ fontSize: '40px', color: 'white' }} />
          </div>

          <Title2 block style={{ marginBottom: tokens.spacingVerticalS }}>
            Scanning Your System...
          </Title2>

          <Body1 block style={{ color: tokens.colorNeutralForeground3, marginBottom: tokens.spacingVerticalL }}>
            {currentTask || 'Initializing scan...'}
          </Body1>

          <ProgressBar
            value={scanProgress / 100}
            thickness="large"
            color="brand"
          />

          <div className={styles.progressInfo}>
            <Caption1>{Math.round(scanProgress)}% Complete</Caption1>
            <Caption1>Please wait...</Caption1>
          </div>
        </Card>
      </div>
    )
  }

  if (hasResults && stats) {
    return (
      <div className={styles.container}>
        <Card>
          <div className={styles.resultsHeader}>
            <div>
              <Title2>Scan Complete</Title2>
              <Caption1 style={{ color: tokens.colorNeutralForeground3 }}>
                Completed in {(stats.duration / 1000).toFixed(1)} seconds
              </Caption1>
            </div>
            <div className={styles.resultsActions}>
              <Button
                appearance="secondary"
                icon={<Play20Regular />}
                onClick={onQuickScan}
              >
                New Scan
              </Button>
              {onCompare && (
                <Button
                  appearance="secondary"
                  icon={<History20Regular />}
                  onClick={onCompare}
                >
                  Compare
                </Button>
              )}
              {onCopyToClipboard && (
                <Button
                  appearance="secondary"
                  icon={<Copy20Regular />}
                  onClick={onCopyToClipboard}
                >
                  Copy
                </Button>
              )}
              {onExport && (
                <Button
                  appearance="primary"
                  icon={<Save20Regular />}
                  onClick={onExport}
                >
                  Export
                </Button>
              )}
            </div>
          </div>

          <div className={styles.statsGrid}>
            <div className={styles.statCard}>
              <Title3 style={{ color: tokens.colorPaletteBlueForeground2 }}>
                {stats.totalTasks}
              </Title3>
              <Caption1>Total Tasks</Caption1>
            </div>
            <div className={styles.statCard}>
              <Title3 style={{ color: tokens.colorPaletteGreenForeground1 }}>
                {stats.successfulTasks}
              </Title3>
              <Caption1>Successful</Caption1>
            </div>
            <div className={styles.statCard}>
              <Title3 style={{ color: stats.failedTasks > 0 ? tokens.colorPaletteRedForeground1 : tokens.colorNeutralForeground3 }}>
                {stats.failedTasks}
              </Title3>
              <Caption1>Failed</Caption1>
            </div>
            <div className={styles.statCard}>
              <Title3 style={{ color: tokens.colorPaletteYellowForeground1 }}>
                {Math.round((stats.successfulTasks / stats.totalTasks) * 100)}%
              </Title3>
              <Caption1>Success Rate</Caption1>
            </div>
          </div>

          <Divider />

          <Body1 style={{ marginTop: tokens.spacingVerticalL }}>
            View detailed results below or export them for further analysis.
          </Body1>
        </Card>
      </div>
    )
  }

  return (
    <div className={styles.container}>
      <Card className={styles.welcomeCard}>
        <div className={styles.iconContainer}>
          <img
            src="/icon.png"
            alt="WF Diagnostics"
            style={{ width: '50px', height: '50px', filter: 'brightness(0) invert(1)' }}
          />
        </div>

        <Title2 block style={{ marginBottom: tokens.spacingVerticalS }}>
          Welcome to System Diagnostics
        </Title2>

        <Body1 block style={{
          color: tokens.colorNeutralForeground2,
          maxWidth: '600px',
          margin: '0 auto'
        }}>
          Choose a scan type to analyze your Windows system. Quick Scan covers essential checks,
          while Full Scan performs comprehensive analysis of all components.
        </Body1>

        <div className={styles.scanButtons}>
          <Button
            className={styles.scanButton}
            appearance="primary"
            icon={<Flash20Regular />}
            onClick={onQuickScan}
          >
            Quick Scan
            <Caption1 style={{ marginLeft: tokens.spacingHorizontalS, opacity: 0.8 }}>
              ~30 seconds
            </Caption1>
          </Button>

          <Button
            className={styles.scanButton}
            appearance="secondary"
            icon={<Search20Regular />}
            onClick={onFullScan}
          >
            Full Scan
            <Caption1 style={{ marginLeft: tokens.spacingHorizontalS, opacity: 0.8 }}>
              3-5 minutes
            </Caption1>
          </Button>
        </div>

        <div className={styles.featureGrid}>
          <div className={styles.featureCard}>
            <CheckmarkCircle20Regular className={styles.featureIcon} style={{ color: tokens.colorPaletteGreenForeground1 }} />
            <Title3>Security Check</Title3>
            <Caption1 style={{ color: tokens.colorNeutralForeground3 }}>
              Analyzes system security settings and configurations
            </Caption1>
          </div>

          <div className={styles.featureCard}>
            <ErrorCircle20Regular className={styles.featureIcon} style={{ color: tokens.colorPaletteBlueForeground2 }} />
            <Title3>Hardware Analysis</Title3>
            <Caption1 style={{ color: tokens.colorNeutralForeground3 }}>
              Checks CPU, RAM, storage, and device health
            </Caption1>
          </div>

          <div className={styles.featureCard}>
            <Clock20Regular className={styles.featureIcon} style={{ color: tokens.colorPalettePurpleForeground2 }} />
            <Title3>Performance Metrics</Title3>
            <Caption1 style={{ color: tokens.colorNeutralForeground3 }}>
              Identifies bottlenecks and optimization opportunities
            </Caption1>
          </div>
        </div>
      </Card>
    </div>
  )
}