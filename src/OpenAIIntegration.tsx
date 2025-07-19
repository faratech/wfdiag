import React, { useState, useRef } from 'react';
import {
  Card,
  Title3,
  Caption1,
  Text,
  Button,
  Textarea,
  Input,
  makeStyles,
  tokens,
  Spinner,
  Badge,
  MessageBar,
  Accordion,
  AccordionItem,
  AccordionHeader,
  AccordionPanel,
  Dialog,
  DialogSurface,
  DialogTitle,
  DialogBody,
  DialogActions,
  DialogContent,
} from '@fluentui/react-components';
import {
  BrainCircuitRegular,
  KeyRegular,
  SendRegular,
  ShieldCheckmarkRegular,
  WarningRegular,
  CheckmarkCircleRegular,
  InfoRegular,
  DismissCircleRegular,
  ArrowLeftRegular,
} from '@fluentui/react-icons';
import { invoke } from '@tauri-apps/api/tauri';

const useStyles = makeStyles({
  container: {
    padding: tokens.spacingVerticalL,
    maxWidth: '1200px',
    margin: '0 auto',
  },
  header: {
    display: 'flex',
    alignItems: 'center',
    gap: tokens.spacingHorizontalM,
    marginBottom: tokens.spacingVerticalL,
  },
  apiKeySection: {
    marginBottom: tokens.spacingVerticalL,
  },
  inputSection: {
    marginBottom: tokens.spacingVerticalL,
  },
  resultsSection: {
    marginTop: tokens.spacingVerticalL,
  },
  findingCard: {
    marginBottom: tokens.spacingVerticalM,
    padding: tokens.spacingVerticalM,
  },
  diagnosticsList: {
    display: 'flex',
    flexWrap: 'wrap',
    gap: tokens.spacingHorizontalS,
    marginTop: tokens.spacingVerticalS,
  },
  recommendation: {
    padding: tokens.spacingVerticalS,
    marginBottom: tokens.spacingVerticalS,
    backgroundColor: tokens.colorNeutralBackground3,
    borderRadius: tokens.borderRadiusMedium,
  },
});

interface OpenAIIntegrationProps {
  sessionId: string;
  onRunDiagnostics?: (taskIds: string[]) => void;
  onBack?: () => void;
}

interface OpenAIResponse {
  analysis: string;
  diagnostics_run: string[];
  findings: Finding[];
  recommendations: string[];
}

interface Finding {
  category: string;
  severity: string;
  description: string;
  details?: string;
}

export const OpenAIIntegration: React.FC<OpenAIIntegrationProps> = ({ onBack }) => {
  const styles = useStyles();
  const [apiKey, setApiKey] = useState('');
  const [showApiKey, setShowApiKey] = useState(false);
  const [prompt, setPrompt] = useState('');
  const [isAnalyzing, setIsAnalyzing] = useState(false);
  const [response, setResponse] = useState<OpenAIResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [showApiKeyDialog, setShowApiKeyDialog] = useState(false);
  const apiKeyInputRef = useRef<HTMLInputElement>(null);

  const handleAnalyze = async () => {
    if (!apiKey) {
      setShowApiKeyDialog(true);
      return;
    }

    if (!prompt.trim()) {
      setError('Please enter a prompt for analysis');
      return;
    }

    setIsAnalyzing(true);
    setError(null);
    setResponse(null);

    try {
      // Use the integrated AI analysis command that handles tool calling
      const result: any = await invoke('analyze_system_with_ai', {
        apiKey: apiKey,
        prompt: prompt
      });

      // Parse the response
      const response: OpenAIResponse = {
        analysis: result.analysis || '',
        diagnostics_run: result.diagnostics_run || [],
        findings: result.findings || [],
        recommendations: result.recommendations || []
      };

      // Store diagnostic results if any were run
      if (result.diagnostic_results) {
        console.log('Diagnostic results:', result.diagnostic_results);
      }

      setResponse(response);
    } catch (err) {
      console.error('OpenAI analysis error:', err);
      if (err instanceof Error) {
        setError(err.message);
      } else if (typeof err === 'string') {
        setError(err);
      } else {
        setError('An error occurred during analysis. Check the console for details.');
      }
    } finally {
      setIsAnalyzing(false);
    }
  };

  const getSeverityIcon = (severity: string) => {
    switch (severity.toLowerCase()) {
      case 'critical':
        return <DismissCircleRegular style={{ color: tokens.colorPaletteRedForeground1 }} />;
      case 'warning':
        return <WarningRegular style={{ color: tokens.colorPaletteYellowForeground1 }} />;
      case 'info':
        return <InfoRegular style={{ color: tokens.colorPaletteBlueForeground2 }} />;
      default:
        return <CheckmarkCircleRegular style={{ color: tokens.colorPaletteGreenForeground1 }} />;
    }
  };

  const getSeverityColor = (severity: string): "danger" | "warning" | "informative" | "success" => {
    switch (severity.toLowerCase()) {
      case 'critical':
        return 'danger';
      case 'warning':
        return 'warning';
      case 'info':
        return 'informative';
      default:
        return 'success';
    }
  };

  const examplePrompts = [
    "Analyze my system for performance issues",
    "Check if my drivers are up to date",
    "Look for disk health problems",
    "Identify any network configuration issues",
    "Find potential security vulnerabilities",
    "Check system stability and reliability"
  ];

  return (
    <div className={styles.container}>
      {/* Header with back button */}
      {onBack && (
        <div style={{ marginBottom: tokens.spacingVerticalM }}>
          <Button 
            appearance="secondary" 
            icon={<ArrowLeftRegular />} 
            onClick={onBack}
          >
            Back to Home
          </Button>
        </div>
      )}
      
      <div className={styles.header}>
        <BrainCircuitRegular fontSize={32} />
        <div>
          <Title3>AI-Powered System Analysis</Title3>
          <Caption1>Use OpenAI to analyze your system and identify issues</Caption1>
        </div>
      </div>

      {/* API Key Section */}
      <Card className={styles.apiKeySection}>
        <div style={{ display: 'flex', alignItems: 'center', gap: tokens.spacingHorizontalM, marginBottom: tokens.spacingVerticalS }}>
          <KeyRegular fontSize={24} />
          <Text weight="semibold">OpenAI API Key</Text>
          <Caption1>(Not saved after closing the app)</Caption1>
        </div>
        <div style={{ display: 'flex', gap: tokens.spacingHorizontalS }}>
          <Input
            ref={apiKeyInputRef}
            type={showApiKey ? 'text' : 'password'}
            value={apiKey}
            onChange={(_, data) => setApiKey(data.value)}
            placeholder="sk-..."
            style={{ flex: 1 }}
          />
          <Button
            appearance="secondary"
            onClick={() => setShowApiKey(!showApiKey)}
          >
            {showApiKey ? 'Hide' : 'Show'}
          </Button>
        </div>
        <MessageBar
          intent="info"
          style={{ marginTop: tokens.spacingVerticalS }}
        >
          Your API key is only stored in memory and will be cleared when you close the app
        </MessageBar>
      </Card>

      {/* Prompt Section */}
      <Card className={styles.inputSection}>
        <Text weight="semibold" style={{ marginBottom: tokens.spacingVerticalS }}>
          What would you like to analyze?
        </Text>
        <Textarea
          value={prompt}
          onChange={(_, data) => setPrompt(data.value)}
          placeholder="Describe what you'd like to investigate..."
          resize="vertical"
          rows={4}
          style={{ marginBottom: tokens.spacingVerticalS }}
        />
        <div style={{ marginBottom: tokens.spacingVerticalM }}>
          <Caption1>Example prompts:</Caption1>
          <div style={{ display: 'flex', flexWrap: 'wrap', gap: tokens.spacingHorizontalXS, marginTop: tokens.spacingVerticalXS }}>
            {examplePrompts.map((example, index) => (
              <Badge
                key={index}
                appearance="tint"
                style={{ cursor: 'pointer' }}
                onClick={() => setPrompt(example)}
              >
                {example}
              </Badge>
            ))}
          </div>
        </div>
        <Button
          appearance="primary"
          icon={<SendRegular />}
          onClick={handleAnalyze}
          disabled={isAnalyzing || !prompt.trim()}
        >
          {isAnalyzing ? 'Analyzing...' : 'Analyze System'}
        </Button>
      </Card>

      {/* Loading State */}
      {isAnalyzing && (
        <Card style={{ textAlign: 'center', padding: tokens.spacingVerticalXL }}>
          <Spinner size="large" />
          <Text style={{ display: 'block', marginTop: tokens.spacingVerticalM }}>
            AI is analyzing your system...
          </Text>
          <Caption1>This may take a few moments</Caption1>
        </Card>
      )}

      {/* Error State */}
      {error && (
        <MessageBar
          intent="error"
          style={{ marginTop: tokens.spacingVerticalM }}
        >
          {error}
        </MessageBar>
      )}

      {/* Results */}
      {response && !isAnalyzing && (
        <div className={styles.resultsSection}>
          {/* Diagnostics Run */}
          {response.diagnostics_run.length > 0 && (
            <Card style={{ marginBottom: tokens.spacingVerticalL }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: tokens.spacingHorizontalS, marginBottom: tokens.spacingVerticalS }}>
                <CheckmarkCircleRegular style={{ color: tokens.colorPaletteGreenForeground1 }} />
                <Text weight="semibold">Diagnostics Executed:</Text>
              </div>
              <div className={styles.diagnosticsList}>
                {response.diagnostics_run.map((taskId, index) => (
                  <Badge key={index} appearance="filled" color="success">
                    {taskId.replace(/_/g, ' ').toUpperCase()}
                  </Badge>
                ))}
              </div>
              <Caption1 style={{ marginTop: tokens.spacingVerticalS }}>
                The AI ran these diagnostic tasks to gather system information
              </Caption1>
            </Card>
          )}

          {/* Findings */}
          {response.findings.length > 0 && (
            <div style={{ marginBottom: tokens.spacingVerticalL }}>
              <Title3 style={{ marginBottom: tokens.spacingVerticalM }}>Findings</Title3>
              <Accordion multiple>
                {response.findings.map((finding, index) => (
                  <AccordionItem key={index} value={`finding-${index}`}>
                    <AccordionHeader>
                      <div style={{ display: 'flex', alignItems: 'center', gap: tokens.spacingHorizontalS }}>
                        {getSeverityIcon(finding.severity)}
                        <Badge appearance="filled" color={getSeverityColor(finding.severity)}>
                          {finding.severity}
                        </Badge>
                        <Badge appearance="tint">{finding.category}</Badge>
                        <Text>{finding.description}</Text>
                      </div>
                    </AccordionHeader>
                    {finding.details && (
                      <AccordionPanel>
                        <Text>{finding.details}</Text>
                      </AccordionPanel>
                    )}
                  </AccordionItem>
                ))}
              </Accordion>
            </div>
          )}

          {/* Recommendations */}
          {response.recommendations.length > 0 && (
            <Card>
              <div style={{ display: 'flex', alignItems: 'center', gap: tokens.spacingHorizontalS, marginBottom: tokens.spacingVerticalM }}>
                <ShieldCheckmarkRegular fontSize={24} />
                <Title3>Recommendations</Title3>
              </div>
              {response.recommendations.map((rec, index) => (
                <div key={index} className={styles.recommendation}>
                  <Text>{rec}</Text>
                </div>
              ))}
            </Card>
          )}

          {/* Full Analysis */}
          <Card style={{ marginTop: tokens.spacingVerticalL }}>
            <Title3 style={{ marginBottom: tokens.spacingVerticalM }}>Full Analysis</Title3>
            <Text style={{ whiteSpace: 'pre-wrap' }}>{response.analysis}</Text>
          </Card>
        </div>
      )}

      {/* API Key Dialog */}
      <Dialog open={showApiKeyDialog} onOpenChange={(_, data) => setShowApiKeyDialog(data.open)}>
        <DialogSurface>
          <DialogBody>
            <DialogTitle>API Key Required</DialogTitle>
            <DialogContent>
              <Text>Please enter your OpenAI API key to use AI analysis.</Text>
              <Text size={200} style={{ marginTop: tokens.spacingVerticalS }}>
                Your key will only be stored in memory and cleared when the app closes.
              </Text>
            </DialogContent>
            <DialogActions>
              <Button appearance="secondary" onClick={() => setShowApiKeyDialog(false)}>
                Cancel
              </Button>
              <Button 
                appearance="primary" 
                onClick={() => {
                  setShowApiKeyDialog(false);
                  apiKeyInputRef.current?.focus();
                }}
              >
                OK
              </Button>
            </DialogActions>
          </DialogBody>
        </DialogSurface>
      </Dialog>
    </div>
  );
};