import React, { useState, useEffect } from 'react';
import {
  Text,
  Button,
  Textarea,
  Input,
  Spinner,
  Badge,
  MessageBar,
} from '@fluentui/react-components';
import {
  SendRegular,
  CheckmarkCircleFilled,
  ErrorCircleFilled,
  WarningFilled,
} from '@fluentui/react-icons';
import { invoke } from '@tauri-apps/api/core';
import * as logger from './utils/logger';
import './styles.css';

interface OpenAIIntegrationProps {
  sessionId: string;
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

interface ConversationEntry {
  role: 'user' | 'assistant';
  content: string;
  timestamp: Date;
  diagnosticsRun?: string[];
}

export const OpenAIIntegration: React.FC<OpenAIIntegrationProps> = ({ sessionId }) => {
  const [apiKey, setApiKey] = useState('');
  const [showApiKey, setShowApiKey] = useState(false);
  const [prompt, setPrompt] = useState('');
  const [isAnalyzing, setIsAnalyzing] = useState(false);
  const [response, setResponse] = useState<OpenAIResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [conversationHistory, setConversationHistory] = useState<ConversationEntry[]>([]);

  // Load API key from secure storage on mount
  useEffect(() => {
    const loadStoredKey = async () => {
      try {
        const storedKey = await invoke('load_api_key') as string;
        if (storedKey) {
          setApiKey(storedKey);
        }
      } catch (err) {
        logger.error('OpenAIIntegration', 'Failed to load stored API key', err);
      }
    };
    loadStoredKey();
  }, []);

  // Save API key to secure storage whenever it changes
  const handleApiKeyChange = async (value: string) => {
    setApiKey(value);
    try {
      if (value) {
        await invoke('store_api_key', { key: value });
      } else {
        await invoke('clear_api_key');
      }
    } catch (err) {
      logger.error('OpenAIIntegration', 'Failed to store API key', err);
    }
  };

  const handleAnalyze = async () => {
    if (!apiKey) {
      setError('Please enter your OpenAI API key');
      return;
    }

    if (!prompt.trim()) {
      setError('Please describe what you want to analyze');
      return;
    }

    setIsAnalyzing(true);
    setError(null);
    setResponse(null);

    // Add user message to history
    const userEntry: ConversationEntry = {
      role: 'user',
      content: prompt,
      timestamp: new Date()
    };
    setConversationHistory(prev => [...prev, userEntry]);

    try {
      const result: any = await invoke('analyze_system_with_ai', {
        apiKey,
        prompt
      });

      const response: OpenAIResponse = {
        analysis: result.analysis || '',
        diagnostics_run: result.diagnostics_run || [],
        findings: result.findings || [],
        recommendations: result.recommendations || []
      };

      setResponse(response);
      
      // Add assistant response to history  
      const assistantEntry: ConversationEntry = {
        role: 'assistant',
        content: response.analysis,
        timestamp: new Date(),
        diagnosticsRun: response.diagnostics_run
      };
      setConversationHistory(prev => [...prev, assistantEntry]);
      
      // Clear the prompt for next message
      setPrompt('');
    } catch (err) {
      logger.error('OpenAIIntegration', 'OpenAI analysis error', err);
      setError(err instanceof Error ? err.message : 'Analysis failed');
    } finally {
      setIsAnalyzing(false);
    }
  };

  const getSeverityIcon = (severity: string) => {
    switch (severity.toLowerCase()) {
      case 'critical':
        return <ErrorCircleFilled fontSize={16} style={{ color: '#ef4444' }} />;
      case 'warning':
        return <WarningFilled fontSize={16} style={{ color: '#f59e0b' }} />;
      default:
        return <CheckmarkCircleFilled fontSize={16} style={{ color: '#10b981' }} />;
    }
  };

  const getSeverityColor = (severity: string): "danger" | "warning" | "success" => {
    switch (severity.toLowerCase()) {
      case 'critical': return 'danger';
      case 'warning': return 'warning';
      default: return 'success';
    }
  };

  const getSeverityBackground = (severity: string) => {
    switch (severity.toLowerCase()) {
      case 'critical': return '#ef4444';
      case 'warning': return '#f59e0b';
      default: return '#10b981';
    }
  };

  return (
    <div style={{ width: '100%', maxWidth: '100%' }}>
      {/* Header */}
      <div className="glass-card" style={{ 
        padding: 20,
        marginBottom: 24,
        background: 'linear-gradient(135deg, rgba(139, 92, 246, 0.1), rgba(236, 72, 153, 0.1))',
        border: '1px solid rgba(139, 92, 246, 0.3)',
      }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 16, marginBottom: 16 }}>
          <div className="category-icon" style={{ 
            background: 'linear-gradient(135deg, #8b5cf6, #ec4899)',
            width: 48,
            height: 48,
          }}>
            <i className="fas fa-brain" style={{ fontSize: 24 }}></i>
          </div>
          <div style={{ flex: 1 }}>
            <Text size={500} weight="bold" style={{ color: '#f1f5f9', display: 'block' }}>
              AI-Powered System Analysis
            </Text>
            <Text size={300} style={{ color: '#94a3b8' }}>
              Get intelligent insights about your system issues
            </Text>
          </div>
        </div>
      </div>

      {/* Current Session Context */}
      {sessionId && (
        <div className="glass-card" style={{ 
          padding: 16, 
          marginBottom: 24,
          background: 'linear-gradient(135deg, rgba(34, 197, 94, 0.1), rgba(59, 130, 246, 0.1))',
          border: '1px solid rgba(34, 197, 94, 0.3)',
        }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
            <div style={{
              width: 32,
              height: 32,
              borderRadius: '50%',
              background: 'linear-gradient(135deg, #22c55e, #3b82f6)',
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'center'
            }}>
              <i className="fas fa-link" style={{ fontSize: 14, color: 'white' }}></i>
            </div>
            <div>
              <Text size={300} weight="semibold" style={{ color: '#22c55e', display: 'block' }}>
                Connected to Diagnostic Session
              </Text>
              <Text size={200} style={{ color: '#94a3b8' }}>
                Session ID: {sessionId}
              </Text>
            </div>
          </div>
        </div>
      )}

      {/* API Key Input */}
      <div className="glass-card" style={{ padding: 20, marginBottom: 24 }}>
        <div style={{ marginBottom: 16 }}>
          <Text size={300} weight="semibold" style={{ color: '#f1f5f9', display: 'block', marginBottom: 8 }}>
            OpenAI API Key
          </Text>
          <div style={{ display: 'flex', gap: 12 }}>
            <Input
              type={showApiKey ? 'text' : 'password'}
              value={apiKey}
              onChange={(e) => handleApiKeyChange(e.target.value)}
              placeholder="sk-..."
              style={{
                flex: 1,
                background: 'rgba(30, 41, 59, 0.5)',
                borderColor: 'rgba(139, 92, 246, 0.3)',
                color: '#f1f5f9',
              }}
            />
            <Button
              appearance="secondary"
              onClick={() => setShowApiKey(!showApiKey)}
              style={{
                background: 'rgba(139, 92, 246, 0.1)',
                border: '1px solid rgba(139, 92, 246, 0.3)',
                color: '#a78bfa',
              }}
            >
              <i className={showApiKey ? "fas fa-eye-slash" : "fas fa-eye"} />
            </Button>
          </div>
          <Text size={200} style={{ color: '#64748b', marginTop: 8, display: 'block' }}>
            Enter your OpenAI API key to enable AI analysis. Get one at{' '}
            <a 
              href="https://platform.openai.com/api-keys" 
              target="_blank" 
              rel="noopener noreferrer"
              style={{ color: '#60a5fa', textDecoration: 'underline' }}
            >
              platform.openai.com
            </a>
          </Text>
        </div>
      </div>

      {/* Analysis Input */}
      <div className="glass-card" style={{ padding: 24, marginBottom: 24, width: '100%' }}>
        <Text size={300} weight="semibold" style={{ color: '#f1f5f9', display: 'block', marginBottom: 16 }}>
          What would you like to analyze?
        </Text>
        
        <Textarea
          value={prompt}
          onChange={(e) => setPrompt(e.target.value)}
          placeholder="Example: 'My computer is running slowly and applications take a long time to open' or 'Check for security issues and system optimization opportunities'"
          style={{
            width: '100%',
            minHeight: 120,
            background: 'rgba(30, 41, 59, 0.5)',
            borderColor: 'rgba(139, 92, 246, 0.3)',
            color: '#f1f5f9',
            marginBottom: 16,
          }}
          disabled={isAnalyzing}
        />

        {error && (
          <MessageBar
            intent="error"
            style={{
              marginBottom: 16,
              background: 'rgba(239, 68, 68, 0.1)',
              borderLeft: '4px solid #ef4444',
            }}
          >
            <Text style={{ color: '#f87171' }}>
              <i className="fas fa-exclamation-triangle" style={{ marginRight: 8 }}></i>
              {error}
            </Text>
          </MessageBar>
        )}

        <Button
          appearance="primary"
          icon={<SendRegular />}
          onClick={handleAnalyze}
          disabled={isAnalyzing || !prompt.trim() || !apiKey}
          size="large"
          style={{
            background: isAnalyzing 
              ? 'rgba(148, 163, 184, 0.3)' 
              : 'linear-gradient(135deg, #8b5cf6, #ec4899)',
            border: 'none',
            color: 'white',
            width: '100%',
          }}
        >
          {isAnalyzing ? (
            <>
              <Spinner size="tiny" style={{ marginRight: 8 }} />
              Analyzing your system...
            </>
          ) : (
            'Analyze System'
          )}
        </Button>
      </div>

      {/* Analysis in Progress Indicator */}
      {isAnalyzing && (
        <div className="glass-card" style={{ 
          padding: 16, 
          marginBottom: 24,
          background: 'linear-gradient(135deg, rgba(59, 130, 246, 0.1), rgba(139, 92, 246, 0.1))',
          border: '1px solid rgba(59, 130, 246, 0.3)',
        }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
            <Spinner size="small" />
            <div>
              <Text size={300} weight="semibold" style={{ color: '#60a5fa', display: 'block' }}>
                AI is analyzing your system...
              </Text>
              <Text size={200} style={{ color: '#94a3b8' }}>
                Running diagnostics and gathering system information. This may take 10-30 seconds.
              </Text>
            </div>
          </div>
        </div>
      )}

      {/* Conversation History */}
      {conversationHistory.length > 0 && (
        <div className="glass-card" style={{ 
          padding: 20, 
          marginBottom: 24,
          maxHeight: 400,
          overflowY: 'auto'
        }}>
          <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: 16 }}>
            <Text size={300} weight="semibold" style={{ color: '#f1f5f9' }}>
              Conversation History
            </Text>
            <Button 
              size="small"
              appearance="subtle"
              onClick={() => setConversationHistory([])}
              style={{ color: '#ef4444' }}
            >
              Clear History
            </Button>
          </div>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
            {conversationHistory.map((entry, index) => (
              <div key={index} style={{
                padding: 12,
                borderRadius: 8,
                background: entry.role === 'user' 
                  ? 'rgba(139, 92, 246, 0.1)' 
                  : 'rgba(16, 185, 129, 0.1)',
                border: `1px solid ${entry.role === 'user' 
                  ? 'rgba(139, 92, 246, 0.3)' 
                  : 'rgba(16, 185, 129, 0.3)'}`
              }}>
                <div style={{ display: 'flex', justifyContent: 'space-between', marginBottom: 8 }}>
                  <Text size={200} weight="semibold" style={{ 
                    color: entry.role === 'user' ? '#a78bfa' : '#34d399' 
                  }}>
                    {entry.role === 'user' ? 'You' : 'Assistant'}
                  </Text>
                  <Text size={100} style={{ color: '#64748b' }}>
                    {entry.timestamp.toLocaleTimeString()}
                  </Text>
                </div>
                <Text size={200} style={{ color: '#e2e8f0', whiteSpace: 'pre-wrap' }}>
                  {entry.content}
                </Text>
                {entry.diagnosticsRun && entry.diagnosticsRun.length > 0 && (
                  <div style={{ marginTop: 8 }}>
                    <Text size={100} style={{ color: '#94a3b8' }}>
                      Diagnostics run: {entry.diagnosticsRun.join(', ')}
                    </Text>
                  </div>
                )}
              </div>
            ))}
          </div>
        </div>
      )}

      {/* Structured Results (No duplicate analysis text) */}
      {response && (
        <>
          {/* Key Findings */}
          {response.findings && response.findings.length > 0 && (
            <div className="glass-card" style={{ padding: 20, marginBottom: 24 }}>
              <Text size={400} weight="semibold" style={{ color: '#f1f5f9', display: 'block', marginBottom: 16 }}>
                <i className="fas fa-search" style={{ marginRight: 8, color: '#ec4899' }}></i>
                Key Findings
              </Text>
              {response.findings.map((finding, index) => (
                <div
                  key={index}
                  style={{
                    padding: 16,
                    marginBottom: 12,
                    background: 'rgba(30, 41, 59, 0.3)',
                    borderRadius: 8,
                    borderLeft: `4px solid ${getSeverityBackground(finding.severity)}`,
                  }}
                >
                  <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 8 }}>
                    {getSeverityIcon(finding.severity)}
                    <Badge
                      appearance="filled"
                      color={getSeverityColor(finding.severity)}
                      style={{
                        background: `${getSeverityBackground(finding.severity)}20`,
                        color: getSeverityBackground(finding.severity),
                      }}
                    >
                      {finding.category}
                    </Badge>
                  </div>
                  <Text size={300} weight="semibold" style={{ color: '#f1f5f9', display: 'block', marginBottom: 4 }}>
                    {finding.description}
                  </Text>
                  {finding.details && (
                    <Text size={200} style={{ color: '#94a3b8' }}>
                      {finding.details}
                    </Text>
                  )}
                </div>
              ))}
            </div>
          )}

          {/* Recommendations */}
          {response.recommendations && response.recommendations.length > 0 && (
            <div className="glass-card" style={{ padding: 20, marginBottom: 24 }}>
              <Text size={400} weight="semibold" style={{ color: '#f1f5f9', display: 'block', marginBottom: 16 }}>
                <i className="fas fa-lightbulb" style={{ marginRight: 8, color: '#fbbf24' }}></i>
                Recommendations
              </Text>
              {response.recommendations.map((rec, index) => (
                <div
                  key={index}
                  style={{
                    padding: 12,
                    marginBottom: 8,
                    background: 'rgba(30, 41, 59, 0.3)',
                    borderRadius: 8,
                    borderLeft: '3px solid #10b981',
                  }}
                >
                  <Text size={300} style={{ color: '#e2e8f0' }}>
                    <i className="fas fa-chevron-right" style={{ marginRight: 8, color: '#10b981' }}></i>
                    {rec}
                  </Text>
                </div>
              ))}
            </div>
          )}

          {/* Diagnostics Performed */}
          {response.diagnostics_run && response.diagnostics_run.length > 0 && (
            <div className="glass-card" style={{ padding: 20 }}>
              <Text size={400} weight="semibold" style={{ color: '#f1f5f9', display: 'block', marginBottom: 16 }}>
                <i className="fas fa-tasks" style={{ marginRight: 8, color: '#3b82f6' }}></i>
                Diagnostics Performed
              </Text>
              <div style={{ display: 'flex', flexWrap: 'wrap', gap: 8 }}>
                {response.diagnostics_run.map((diag, index) => (
                  <Badge
                    key={index}
                    appearance="filled"
                    style={{
                      background: 'rgba(59, 130, 246, 0.2)',
                      color: '#60a5fa',
                      border: '1px solid rgba(59, 130, 246, 0.3)',
                    }}
                  >
                    {diag}
                  </Badge>
                ))}
              </div>
            </div>
          )}
        </>
      )}

    </div>
  );
};