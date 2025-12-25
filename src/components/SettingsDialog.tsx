import React, { useState } from 'react'
import { open as openDialog } from '@tauri-apps/plugin-dialog'
import {
  Dialog,
  DialogTrigger,
  DialogSurface,
  DialogTitle,
  DialogBody,
  DialogActions,
  DialogContent,
  Button,
  Field,
  Input,
  Switch,
  makeStyles,
  tokens,
  shorthands,
  Label,
  Caption1,
  Divider,
  RadioGroup,
  Radio,
  SpinButton
} from '@fluentui/react-components'
import {
  Settings20Regular,
  Save20Regular,
  Dismiss20Regular,
  Key20Regular,
  Folder20Regular
} from '@fluentui/react-icons'

const useStyles = makeStyles({
  surface: {
    maxWidth: '600px',
    width: '90vw',
  },

  content: {
    display: 'flex',
    flexDirection: 'column',
    ...shorthands.gap(tokens.spacingVerticalL),
  },

  section: {
    display: 'flex',
    flexDirection: 'column',
    ...shorthands.gap(tokens.spacingVerticalM),
  },

  sectionTitle: {
    fontSize: tokens.fontSizeBase400,
    fontWeight: tokens.fontWeightSemibold,
    color: tokens.colorNeutralForeground1,
    marginBottom: tokens.spacingVerticalS,
  },

  field: {
    display: 'flex',
    flexDirection: 'column',
    ...shorthands.gap(tokens.spacingVerticalXS),
  },

  switchField: {
    display: 'flex',
    justifyContent: 'space-between',
    alignItems: 'center',
    ...shorthands.padding(tokens.spacingVerticalS, 0),
  },

  hint: {
    fontSize: tokens.fontSizeBase200,
    color: tokens.colorNeutralForeground3,
  }
})

export interface SettingsData {
  openAiApiKey?: string
  autoSave?: boolean
  scanOnStartup?: boolean
  maxConcurrentTasks?: number
  exportFormat?: 'json' | 'text' | 'html'
  theme?: 'dark' | 'light' | 'auto'
  showNotifications?: boolean
  customExportPath?: string
  retainHistory?: boolean
  historyLimit?: number
}

export interface SettingsDialogProps {
  trigger?: React.ReactNode
  settings?: SettingsData
  onSave?: (settings: SettingsData) => void
  open?: boolean
  onOpenChange?: (open: boolean) => void
}

export const SettingsDialog: React.FC<SettingsDialogProps> = ({
  trigger,
  settings: initialSettings = {},
  onSave,
  open,
  onOpenChange
}) => {
  const styles = useStyles()
  const [settings, setSettings] = useState<SettingsData>({
    autoSave: true,
    scanOnStartup: false,
    maxConcurrentTasks: 5,
    exportFormat: 'text',
    theme: 'dark',
    showNotifications: true,
    retainHistory: true,
    historyLimit: 30,
    ...initialSettings
  })

  const handleSave = () => {
    onSave?.(settings)
    onOpenChange?.(false)
  }

  const updateSetting = <K extends keyof SettingsData>(
    key: K,
    value: SettingsData[K]
  ) => {
    setSettings(prev => ({ ...prev, [key]: value }))
  }

  const dialogContent = (
    <>
      <DialogBody>
        <DialogTitle>
          <Settings20Regular style={{ marginRight: tokens.spacingHorizontalS }} />
          Settings
        </DialogTitle>
        <DialogContent className={styles.content}>
          {/* API Configuration */}
          <div className={styles.section}>
            <Label className={styles.sectionTitle}>API Configuration</Label>
            <Field
              label="OpenAI API Key"
              hint="Required for AI-powered analysis features"
            >
              <Input
                type="password"
                value={settings.openAiApiKey || ''}
                onChange={(_, data) => updateSetting('openAiApiKey', data.value)}
                placeholder="sk-..."
                contentBefore={<Key20Regular />}
              />
            </Field>
          </div>

          <Divider />

          {/* Scan Settings */}
          <div className={styles.section}>
            <Label className={styles.sectionTitle}>Scan Settings</Label>

            <div className={styles.switchField}>
              <div>
                <Label>Scan on Startup</Label>
                <Caption1 className={styles.hint}>
                  Automatically run a quick scan when the app starts
                </Caption1>
              </div>
              <Switch
                checked={settings.scanOnStartup}
                onChange={(_, data) => updateSetting('scanOnStartup', data.checked)}
              />
            </div>

            <Field
              label="Maximum Concurrent Tasks"
              hint="Number of diagnostic tasks to run in parallel (1-10)"
            >
              <SpinButton
                value={settings.maxConcurrentTasks}
                onChange={(_, data) => updateSetting('maxConcurrentTasks', data.value || 5)}
                min={1}
                max={10}
                appearance="underline"
              />
            </Field>
          </div>

          <Divider />

          {/* Export Settings */}
          <div className={styles.section}>
            <Label className={styles.sectionTitle}>Export Settings</Label>

            <div className={styles.switchField}>
              <div>
                <Label>Auto-save Results</Label>
                <Caption1 className={styles.hint}>
                  Automatically save scan results to history
                </Caption1>
              </div>
              <Switch
                checked={settings.autoSave}
                onChange={(_, data) => updateSetting('autoSave', data.checked)}
              />
            </div>

            <Field label="Default Export Format">
              <RadioGroup
                value={settings.exportFormat}
                onChange={(_, data) => updateSetting('exportFormat', data.value as any)}
              >
                <Radio value="text" label="Plain Text (.txt)" />
                <Radio value="json" label="JSON (.json)" />
                <Radio value="html" label="HTML Report (.html)" />
              </RadioGroup>
            </Field>

            <Field
              label="Custom Export Path"
              hint="Leave empty to use default Downloads folder"
            >
              <div style={{ display: 'flex', gap: '8px' }}>
                <Input
                  style={{ flex: 1 }}
                  value={settings.customExportPath || ''}
                  onChange={(_, data) => updateSetting('customExportPath', data.value)}
                  placeholder="C:\Users\...\Documents"
                  contentBefore={<Folder20Regular />}
                />
                <Button
                  appearance="secondary"
                  onClick={async () => {
                    try {
                      const selected = await openDialog({
                        directory: true,
                        multiple: false,
                        title: 'Select Export Folder'
                      })
                      if (selected && typeof selected === 'string') {
                        updateSetting('customExportPath', selected)
                      }
                    } catch (error) {
                      console.error('Failed to open folder picker:', error)
                    }
                  }}
                >
                  Browse
                </Button>
              </div>
            </Field>
          </div>

          <Divider />

          {/* Appearance */}
          <div className={styles.section}>
            <Label className={styles.sectionTitle}>Appearance</Label>

            <Field label="Theme">
              <RadioGroup
                value={settings.theme}
                onChange={(_, data) => updateSetting('theme', data.value as any)}
              >
                <Radio value="dark" label="Dark" />
                <Radio value="light" label="Light" />
                <Radio value="auto" label="System Default" />
              </RadioGroup>
            </Field>

            <div className={styles.switchField}>
              <div>
                <Label>Show Notifications</Label>
                <Caption1 className={styles.hint}>
                  Display system notifications for important events
                </Caption1>
              </div>
              <Switch
                checked={settings.showNotifications}
                onChange={(_, data) => updateSetting('showNotifications', data.checked)}
              />
            </div>
          </div>

          <Divider />

          {/* History Settings */}
          <div className={styles.section}>
            <Label className={styles.sectionTitle}>History</Label>

            <div className={styles.switchField}>
              <div>
                <Label>Retain Scan History</Label>
                <Caption1 className={styles.hint}>
                  Keep previous scan results for comparison
                </Caption1>
              </div>
              <Switch
                checked={settings.retainHistory}
                onChange={(_, data) => updateSetting('retainHistory', data.checked)}
              />
            </div>

            {settings.retainHistory && (
              <Field
                label="History Limit (days)"
                hint="Automatically remove scans older than this"
              >
                <SpinButton
                  value={settings.historyLimit}
                  onChange={(_, data) => updateSetting('historyLimit', data.value || 30)}
                  min={1}
                  max={365}
                  appearance="underline"
                />
              </Field>
            )}
          </div>

        </DialogContent>
        <DialogActions>
          <DialogTrigger disableButtonEnhancement>
            <Button appearance="secondary" icon={<Dismiss20Regular />}>
              Cancel
            </Button>
          </DialogTrigger>
          <Button appearance="primary" icon={<Save20Regular />} onClick={handleSave}>
            Save Changes
          </Button>
        </DialogActions>
      </DialogBody>
    </>
  )

  // If we have a trigger, wrap in Dialog with DialogTrigger
  if (trigger) {
    return (
      <Dialog open={open} onOpenChange={(_, data) => onOpenChange?.(data.open)}>
        <DialogTrigger disableButtonEnhancement>{trigger as React.ReactElement}</DialogTrigger>
        <DialogSurface className={styles.surface}>
          {dialogContent}
        </DialogSurface>
      </Dialog>
    )
  }

  // Otherwise just render the Dialog without trigger
  return (
    <Dialog open={open} onOpenChange={(_, data) => onOpenChange?.(data.open)}>
      <DialogSurface className={styles.surface}>
        {dialogContent}
      </DialogSurface>
    </Dialog>
  )
}