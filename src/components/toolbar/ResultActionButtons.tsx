/**
 * Shared result action buttons used by CommandBar and CompactToolbar
 */

import React from 'react'
import {
  Button,
  Tooltip,
  ToolbarButton,
} from '@fluentui/react-components'
import {
  Save16Regular,
  Save20Regular,
  Copy16Regular,
  Copy20Regular,
  History16Regular,
  History20Regular,
  Delete16Regular,
  Delete20Regular,
} from '@fluentui/react-icons'

export interface ResultActionButtonsProps {
  onExport?: () => void
  onCopy?: () => void
  onCompare?: () => void
  onClear?: () => void
  disabled?: boolean
  variant?: 'compact' | 'full'
  showLabels?: boolean
}

/**
 * Compact variant - small icon buttons
 */
const CompactResultButtons: React.FC<ResultActionButtonsProps> = ({
  onExport,
  onCopy,
  onCompare,
  onClear,
  disabled = false,
}) => (
  <>
    {onCompare && (
      <Tooltip content="Compare with previous scan" relationship="label">
        <Button
          appearance="subtle"
          size="small"
          icon={<History16Regular />}
          onClick={onCompare}
          disabled={disabled}
        />
      </Tooltip>
    )}

    {onCopy && (
      <Tooltip content="Copy results to clipboard" relationship="label">
        <Button
          appearance="subtle"
          size="small"
          icon={<Copy16Regular />}
          onClick={onCopy}
          disabled={disabled}
        />
      </Tooltip>
    )}

    {onExport && (
      <Tooltip content="Export results" relationship="label">
        <Button
          appearance="subtle"
          size="small"
          icon={<Save16Regular />}
          onClick={onExport}
          disabled={disabled}
        />
      </Tooltip>
    )}

    {onClear && (
      <Tooltip content="Clear results" relationship="label">
        <Button
          appearance="subtle"
          size="small"
          icon={<Delete16Regular />}
          onClick={onClear}
          disabled={disabled}
        />
      </Tooltip>
    )}
  </>
)

/**
 * Full variant - toolbar buttons with optional labels
 */
const FullResultButtons: React.FC<ResultActionButtonsProps> = ({
  onExport,
  onCopy,
  onCompare,
  onClear,
  disabled = false,
}) => (
  <>
    {onCompare && (
      <Tooltip content="Compare with previous scan" relationship="description">
        <ToolbarButton
          appearance="subtle"
          icon={<History20Regular />}
          onClick={onCompare}
          disabled={disabled}
          aria-label="Compare scans"
        />
      </Tooltip>
    )}

    {onExport && (
      <Tooltip content="Export results" relationship="description">
        <ToolbarButton
          appearance="subtle"
          icon={<Save20Regular />}
          onClick={onExport}
          disabled={disabled}
          aria-label="Export"
        />
      </Tooltip>
    )}

    {onCopy && (
      <Tooltip content="Copy to clipboard" relationship="description">
        <ToolbarButton
          appearance="subtle"
          icon={<Copy20Regular />}
          onClick={onCopy}
          disabled={disabled}
          aria-label="Copy"
        />
      </Tooltip>
    )}

    {onClear && (
      <Tooltip content="Clear results" relationship="description">
        <ToolbarButton
          appearance="subtle"
          icon={<Delete20Regular />}
          onClick={onClear}
          disabled={disabled}
          aria-label="Clear results"
        />
      </Tooltip>
    )}
  </>
)

export const ResultActionButtons: React.FC<ResultActionButtonsProps> = (props) => {
  const { variant = 'compact' } = props

  if (variant === 'full') {
    return <FullResultButtons {...props} />
  }

  return <CompactResultButtons {...props} />
}

export default ResultActionButtons
