import React from 'react'
import {
  Dialog,
  DialogSurface,
  DialogTitle,
  DialogBody,
  DialogContent,
  Button,
  makeStyles,
  tokens,
  shorthands,
  Text,
  Divider,
  Badge
} from '@fluentui/react-components'
import {
  Dismiss24Regular,
  Info20Regular,
  Heart20Regular,
  Globe20Regular,
  Mail20Regular,
  Shield20Regular
} from '@fluentui/react-icons'

const useStyles = makeStyles({
  surface: {
    maxWidth: '500px',
    background: 'linear-gradient(135deg, rgba(16, 23, 41, 0.98), rgba(26, 34, 56, 0.95))',
    backdropFilter: 'blur(20px)',
    ...shorthands.border('1px', 'solid', 'rgba(139, 158, 255, 0.1)'),
  },

  header: {
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'center',
    flexDirection: 'column',
    ...shorthands.padding(tokens.spacingVerticalXL),
    background: 'radial-gradient(ellipse at center, rgba(79, 105, 255, 0.1), transparent 70%)',
  },

  logo: {
    width: '80px',
    height: '80px',
    ...shorthands.borderRadius('50%'),
    background: 'linear-gradient(135deg, #4F69FF, #6B7FFF)',
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'center',
    marginBottom: tokens.spacingVerticalM,
    boxShadow: '0 8px 24px rgba(79, 105, 255, 0.3)',
  },

  title: {
    fontSize: tokens.fontSizeBase600,
    fontWeight: tokens.fontWeightBold,
    background: 'linear-gradient(135deg, #6B7FFF, #8B9EFF)',
    WebkitBackgroundClip: 'text',
    WebkitTextFillColor: 'transparent',
    marginBottom: tokens.spacingVerticalXS,
  },

  version: {
    display: 'flex',
    alignItems: 'center',
    ...shorthands.gap(tokens.spacingHorizontalS),
    marginBottom: tokens.spacingVerticalL,
  },

  section: {
    marginBottom: tokens.spacingVerticalL,
  },

  sectionTitle: {
    fontSize: tokens.fontSizeBase400,
    fontWeight: tokens.fontWeightSemibold,
    marginBottom: tokens.spacingVerticalS,
    color: tokens.colorBrandForeground1,
  },

  description: {
    lineHeight: 1.6,
    color: tokens.colorNeutralForeground2,
    marginBottom: tokens.spacingVerticalM,
  },

  features: {
    display: 'grid',
    ...shorthands.gap(tokens.spacingVerticalS),
  },

  feature: {
    display: 'flex',
    alignItems: 'flex-start',
    ...shorthands.gap(tokens.spacingHorizontalS),
  },

  featureIcon: {
    color: tokens.colorBrandForeground1,
    marginTop: '2px',
  },

  links: {
    display: 'flex',
    ...shorthands.gap(tokens.spacingHorizontalL),
    justifyContent: 'center',
    marginTop: tokens.spacingVerticalM,
  },

  link: {
    display: 'flex',
    alignItems: 'center',
    ...shorthands.gap(tokens.spacingHorizontalXS),
    color: tokens.colorNeutralForeground2,
  },

  footer: {
    textAlign: 'center',
    ...shorthands.padding(tokens.spacingVerticalM, 0),
    color: tokens.colorNeutralForeground3,
    fontSize: tokens.fontSizeBase200,
  },
})

export interface AboutDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
}

export const AboutDialog: React.FC<AboutDialogProps> = ({ open, onOpenChange }) => {
  const styles = useStyles()

  const features = [
    '100+ comprehensive system diagnostic checks',
    'Real-time system monitoring and analysis',
    'AI-powered insights with OpenAI integration',
    'Scan history and comparison tools',
    'Multiple export formats (JSON, HTML, Text)',
    'Windows 10/11 optimized performance',
  ]

  return (
    <Dialog open={open} onOpenChange={(_, data) => onOpenChange(data.open)}>
      <DialogSurface className={styles.surface}>
        <DialogBody>
          <DialogTitle action={
            <Button
              appearance="subtle"
              aria-label="close"
              icon={<Dismiss24Regular />}
              onClick={() => onOpenChange(false)}
            />
          }>
            About WindowsForum Diagnostics
          </DialogTitle>

          <DialogContent>
            <div className={styles.header}>
              <div className={styles.logo}>
                <img
                  src="/icon.png"
                  alt="WF Diagnostics"
                  style={{ width: '60px', height: '60px' }}
                />
              </div>
              <Text className={styles.title}>WindowsForum Diagnostics</Text>
              <div className={styles.version}>
                <Badge appearance="filled" color="brand" size="medium">
                  Version 2.1.5
                </Badge>
                <Badge appearance="tint" color="success" size="medium">
                  <Shield20Regular style={{ marginRight: '4px' }} />
                  Professional Edition
                </Badge>
              </div>
            </div>

            <Divider />

            <div className={styles.section}>
              <Text className={styles.description}>
                A professional Windows system diagnostic tool designed for IT professionals,
                system administrators, and power users. Get comprehensive insights into your
                Windows system with advanced diagnostics, real-time monitoring, and AI-powered analysis.
              </Text>
            </div>

            <div className={styles.section}>
              <Text className={styles.sectionTitle}>Key Features</Text>
              <div className={styles.features}>
                {features.map((feature, index) => (
                  <div key={index} className={styles.feature}>
                    <Info20Regular className={styles.featureIcon} />
                    <Text size={200}>{feature}</Text>
                  </div>
                ))}
              </div>
            </div>

            <Divider />

            <div className={styles.links}>
              <div className={styles.link} style={{ cursor: 'default' }}>
                <Globe20Regular />
                <Text>WindowsForum.com</Text>
              </div>
              <div className={styles.link} style={{ cursor: 'default' }}>
                <Heart20Regular />
                <Text>Community Forum</Text>
              </div>
              <div className={styles.link} style={{ cursor: 'default' }}>
                <Mail20Regular />
                <Text>Support Available</Text>
              </div>
            </div>

            <div className={styles.footer}>
              <Text>Built with React, Tauri, and Fluent UI</Text>
              <br />
              <Text style={{ marginTop: tokens.spacingVerticalS }}>
                Made with <Heart20Regular style={{ color: '#EF4444', verticalAlign: 'middle' }} /> by the WindowsForum Team
              </Text>
            </div>
          </DialogContent>
        </DialogBody>
      </DialogSurface>
    </Dialog>
  )
}