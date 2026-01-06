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
    backgroundColor: tokens.colorNeutralBackground1,
    ...shorthands.border('1px', 'solid', tokens.colorNeutralStroke1),
    ...shorthands.padding(tokens.spacingVerticalXXXL),
    textAlign: 'center',
    boxShadow: tokens.shadow8,
    position: 'relative',
    ...shorthands.overflow('hidden'),
  },

  iconContainer: {
    width: '72px',
    height: '72px',
    ...shorthands.margin('0', 'auto', tokens.spacingVerticalXL),
    ...shorthands.borderRadius('50%'),
    backgroundColor: tokens.colorBrandBackground,
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'center',
    boxShadow: tokens.shadow16,
    position: 'relative',
    zIndex: 1,
  },

  scanButtons: {
    display: 'flex',
    justifyContent: 'center',
    flexWrap: 'wrap',
    ...shorthands.gap(tokens.spacingHorizontalL),
    ...shorthands.margin(tokens.spacingVerticalXL, '0'),
    '@media (max-width: 640px)': {
      flexDirection: 'column',
      alignItems: 'stretch',
      ...shorthands.gap(tokens.spacingVerticalM),
    },
  },

  scanButton: {
    minWidth: '180px',
    height: '48px',
    fontSize: tokens.fontSizeBase400,
    '@media (max-width: 640px)': {
      minWidth: 'unset',
      width: '100%',
    },
  },

  featureGrid: {
    display: 'grid',
    gridTemplateColumns: 'repeat(auto-fit, minmax(300px, 1fr))',
    ...shorthands.gap(tokens.spacingHorizontalL),
    marginTop: tokens.spacingVerticalXXL,
  },

  featureCard: {
    ...shorthands.padding(tokens.spacingVerticalXL),
    backgroundColor: tokens.colorNeutralBackground2,
    ...shorthands.border('1px', 'solid', tokens.colorNeutralStroke1),
    ...shorthands.borderRadius(tokens.borderRadiusLarge),
    boxShadow: tokens.shadow4,
    transitionProperty: 'background, box-shadow, transform',
    transitionDuration: tokens.durationNormal,
    transitionTimingFunction: tokens.curveEasyEase,
    position: 'relative',
    zIndex: 1,

    ':hover': {
      backgroundColor: tokens.colorNeutralBackground1Hover,
      transform: 'translateY(-4px)',
      boxShadow: tokens.shadow16,
    }
  },

  featureIcon: {
    fontSize: '24px',
    marginBottom: tokens.spacingVerticalM,
  },

  progressCard: {
    ...shorthands.padding(tokens.spacingVerticalXL),
    textAlign: 'center',
  },

  progressInfo: {
    display: 'flex',
    justifyContent: 'space-between',
    marginTop: tokens.spacingVerticalL,
  },

  resultsHeader: {
    display: 'flex',
    justifyContent: 'space-between',
    alignItems: 'center',
    flexWrap: 'wrap',
    ...shorthands.gap(tokens.spacingVerticalM),
    marginBottom: tokens.spacingVerticalXL,
    '@media (max-width: 640px)': {
      flexDirection: 'column',
      alignItems: 'stretch',
    },
  },

  resultsActions: {
    display: 'flex',
    flexWrap: 'wrap',
    ...shorthands.gap(tokens.spacingHorizontalS),
    '@media (max-width: 640px)': {
      justifyContent: 'stretch',
      '& > button': {
        flex: 1,
      },
    },
  },

  statsGrid: {
    display: 'grid',
    gridTemplateColumns: 'repeat(4, 1fr)',
    ...shorthands.gap(tokens.spacingHorizontalM),
    marginBottom: tokens.spacingVerticalL,
    '@media (max-width: 768px)': {
      gridTemplateColumns: 'repeat(2, 1fr)',
    },
    '@media (max-width: 480px)': {
      gridTemplateColumns: '1fr',
    },
  },

  statCard: {
    ...shorthands.padding(tokens.spacingVerticalM),
    backgroundColor: tokens.colorNeutralBackground2,
    ...shorthands.border('1px', 'solid', tokens.colorNeutralStroke1),
    ...shorthands.borderRadius(tokens.borderRadiusMedium),
    boxShadow: tokens.shadow2,
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
  /** Current task number (1-based) */
  currentTaskNumber?: number
  /** Total number of tasks in scan */
  totalTasks?: number
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
  currentTaskNumber = 0,
  totalTasks = 0,
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

          <Title2 block style={{ marginBottom: tokens.spacingVerticalM }}>
            Scanning Your System...
          </Title2>

          <Body1 block style={{ color: tokens.colorNeutralForeground3, marginBottom: tokens.spacingVerticalS }}>
            {currentTaskNumber > 0 && totalTasks > 0
              ? `Task ${currentTaskNumber} of ${totalTasks}`
              : 'Initializing scan...'}
          </Body1>

          <Caption1 block style={{ color: tokens.colorNeutralForeground2, marginBottom: tokens.spacingVerticalXL }}>
            {currentTask || 'Preparing diagnostics...'}
          </Caption1>

          <ProgressBar
            value={scanProgress / 100}
            thickness="large"
            color="brand"
          />

          <div className={styles.progressInfo}>
            <Caption1>{Math.round(scanProgress)}% Complete</Caption1>
            <Caption1>
              {totalTasks > 0
                ? `${totalTasks - currentTaskNumber} tasks remaining`
                : 'Please wait...'}
            </Caption1>
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
              <Caption1>Checks Run</Caption1>
            </div>
            <div className={styles.statCard}>
              <Title3 style={{ color: tokens.colorPaletteGreenForeground1 }}>
                {stats.successfulTasks}
              </Title3>
              <Caption1>Passed</Caption1>
            </div>
            <div className={styles.statCard}>
              <Title3 style={{ color: stats.failedTasks > 0 ? tokens.colorPaletteRedForeground1 : tokens.colorNeutralForeground3 }}>
                {stats.failedTasks}
              </Title3>
              <Caption1>Need Attention</Caption1>
            </div>
            <div className={styles.statCard}>
              <Title3 style={{ color: tokens.colorPaletteYellowForeground1 }}>
                {Math.round((stats.successfulTasks / stats.totalTasks) * 100)}%
              </Title3>
              <Caption1>Health Score</Caption1>
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
            style={{ width: '50px', height: '50px', objectFit: 'contain' }}
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
          Analyze your Windows system for issues, performance bottlenecks, and configuration problems.
        </Body1>

        <div className={styles.scanButtons}>
          <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', gap: tokens.spacingVerticalS }}>
            <Button
              className={styles.scanButton}
              appearance="primary"
              icon={<Flash20Regular />}
              onClick={onQuickScan}
            >
              Quick Scan
              <Caption1 style={{ marginLeft: tokens.spacingHorizontalS, opacity: 0.8 }}>
                ~30 sec
              </Caption1>
            </Button>
            <Caption1 style={{ color: tokens.colorNeutralForeground3, textAlign: 'center', maxWidth: '200px' }}>
              Essential checks: drivers, disk space, memory, and critical services
            </Caption1>
          </div>

          <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', gap: tokens.spacingVerticalS }}>
            <Button
              className={styles.scanButton}
              appearance="secondary"
              icon={<Search20Regular />}
              onClick={onFullScan}
            >
              Full Scan
              <Caption1 style={{ marginLeft: tokens.spacingHorizontalS, opacity: 0.8 }}>
                3-5 min
              </Caption1>
            </Button>
            <Caption1 style={{ color: tokens.colorNeutralForeground3, textAlign: 'center', maxWidth: '200px' }}>
              Complete analysis: hardware, software, network, logs, and security
            </Caption1>
          </div>
        </div>

        <div className={styles.featureGrid}>
          <div className={styles.featureCard}>
            <CheckmarkCircle20Regular className={styles.featureIcon} style={{ color: tokens.colorPaletteGreenForeground1 }} />
            <Title3 style={{ marginBottom: tokens.spacingVerticalS }}>Security Check</Title3>
            <Caption1 style={{ color: tokens.colorNeutralForeground3, display: 'block', marginTop: tokens.spacingVerticalXS }}>
              Analyzes system security settings and configurations
            </Caption1>
          </div>

          <div className={styles.featureCard}>
            <ErrorCircle20Regular className={styles.featureIcon} style={{ color: tokens.colorPaletteBlueForeground2 }} />
            <Title3 style={{ marginBottom: tokens.spacingVerticalS }}>Hardware Analysis</Title3>
            <Caption1 style={{ color: tokens.colorNeutralForeground3, display: 'block', marginTop: tokens.spacingVerticalXS }}>
              Checks CPU, RAM, storage, and device health
            </Caption1>
          </div>

          <div className={styles.featureCard}>
            <Clock20Regular className={styles.featureIcon} style={{ color: tokens.colorPalettePurpleForeground2 }} />
            <Title3 style={{ marginBottom: tokens.spacingVerticalS }}>Performance Metrics</Title3>
            <Caption1 style={{ color: tokens.colorNeutralForeground3, display: 'block', marginTop: tokens.spacingVerticalXS }}>
              Identifies bottlenecks and optimization opportunities
            </Caption1>
          </div>
        </div>
      </Card>
    </div>
  )
}