import React, { useState, useEffect } from 'react';
import {
  Button,
  Spinner,
  Text,
  Card,
  makeStyles,
  tokens,
} from '@fluentui/react-components';
import {
  PersonLockRegular,
  CheckmarkCircleFilled,
  ErrorCircleFilled,
  ArrowSyncRegular,
} from '@fluentui/react-icons';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-shell';
import * as logger from './utils/logger';

const useStyles = makeStyles({
  container: {
    display: 'flex',
    flexDirection: 'column',
    gap: tokens.spacingVerticalL,
    padding: tokens.spacingVerticalXL,
  },
  card: {
    padding: tokens.spacingVerticalL,
    background: 'rgba(30, 41, 59, 0.5)',
    border: '1px solid rgba(139, 92, 246, 0.3)',
  },
  statusCard: {
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'space-between',
    padding: tokens.spacingVerticalM,
  },
  statusInfo: {
    display: 'flex',
    alignItems: 'center',
    gap: tokens.spacingHorizontalM,
  },
  userInfo: {
    display: 'flex',
    flexDirection: 'column',
    gap: tokens.spacingVerticalXS,
  },
  error: {
    color: tokens.colorPaletteRedForeground1,
    marginTop: tokens.spacingVerticalS,
  },
  success: {
    color: tokens.colorPaletteGreenForeground1,
  },
  warning: {
    color: tokens.colorPaletteYellowForeground1,
  },
});

interface OAuthUserInfo {
  id: number;
  username: string;
  email?: string;
  avatar_url?: string;
  groups: string[];
}

interface OAuthLoginProps {
  onAuthChange?: (isAuthenticated: boolean, userInfo?: OAuthUserInfo) => void;
}

export const OAuthLogin: React.FC<OAuthLoginProps> = ({ onAuthChange }) => {
  const styles = useStyles();
  const [isAuthenticated, setIsAuthenticated] = useState(false);
  const [userInfo, setUserInfo] = useState<OAuthUserInfo | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [isRefreshing, setIsRefreshing] = useState(false);

  useEffect(() => {
    checkAuthStatus();
    
    // Listen for OAuth callback events
    const unlistenCallback = listen('oauth-callback', async (event: any) => {
      const { code, state } = event.payload;
      await handleOAuthCallback(code, state);
    });

    const unlistenError = listen('oauth-error', (event: any) => {
      const { error, description } = event.payload;
      setError(description || error);
      setIsLoading(false);
    });

    return () => {
      unlistenCallback.then(fn => fn());
      unlistenError.then(fn => fn());
    };
  }, []);

  const checkAuthStatus = async () => {
    try {
      const [authenticated, info] = await invoke('oauth_get_status') as [boolean, OAuthUserInfo | null];
      setIsAuthenticated(authenticated);
      setUserInfo(info);
      onAuthChange?.(authenticated, info || undefined);
    } catch (err) {
      logger.error('OAuthLogin', 'Failed to check OAuth status', err);
    }
  };

  const startOAuthFlow = async () => {
    setIsLoading(true);
    setError(null);

    try {
      // Get authorization URL
      const authUrl = await invoke('oauth_start_flow') as string;
      
      // Open in default browser
      await open(authUrl);
      
      // The user will authenticate in the browser
      // We'll receive the callback through the event listener
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to start authentication');
      setIsLoading(false);
    }
  };

  const handleOAuthCallback = async (code: string, state: string) => {
    try {
      // Exchange code for token
      await invoke('oauth_handle_callback', { code, callbackState: state });
      
      // Get updated auth status
      await checkAuthStatus();
      
      setIsLoading(false);
      setError(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Authentication failed');
      setIsLoading(false);
    }
  };

  const handleLogout = async () => {
    try {
      await invoke('oauth_logout');
      setIsAuthenticated(false);
      setUserInfo(null);
      onAuthChange?.(false);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Logout failed');
    }
  };

  const refreshToken = async () => {
    setIsRefreshing(true);
    try {
      await invoke('oauth_refresh_token');
      await checkAuthStatus();
      setError(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Token refresh failed');
    } finally {
      setIsRefreshing(false);
    }
  };

  return (
    <div className={styles.container}>
      <Card className={styles.card}>
        <div className={styles.statusCard}>
          <div className={styles.statusInfo}>
            {isAuthenticated ? (
              <>
                <CheckmarkCircleFilled 
                  fontSize={24} 
                  className={styles.success}
                />
                <div className={styles.userInfo}>
                  <Text size={400} weight="semibold" style={{ color: '#f1f5f9' }}>
                    Connected to WindowsForum
                  </Text>
                  <Text size={300} style={{ color: '#94a3b8' }}>
                    Logged in as: {userInfo?.username}
                  </Text>
                  {userInfo?.email && (
                    <Text size={200} style={{ color: '#64748b' }}>
                      {userInfo.email}
                    </Text>
                  )}
                  {userInfo?.groups && userInfo.groups.length > 0 && (
                    <Text size={200} style={{ color: '#64748b' }}>
                      Groups: {userInfo.groups.join(', ')}
                    </Text>
                  )}
                </div>
              </>
            ) : (
              <>
                <PersonLockRegular 
                  fontSize={24} 
                  className={styles.warning}
                />
                <div className={styles.userInfo}>
                  <Text size={400} weight="semibold" style={{ color: '#f1f5f9' }}>
                    Authentication Required
                  </Text>
                  <Text size={300} style={{ color: '#94a3b8' }}>
                    Sign in with your WindowsForum account to continue
                  </Text>
                </div>
              </>
            )}
          </div>
          
          <div style={{ display: 'flex', gap: 8 }}>
            {isAuthenticated && (
              <Button
                appearance="secondary"
                icon={<ArrowSyncRegular />}
                onClick={refreshToken}
                disabled={isRefreshing}
                style={{
                  background: 'rgba(59, 130, 246, 0.1)',
                  border: '1px solid rgba(59, 130, 246, 0.3)',
                  color: '#60a5fa',
                }}
              >
                {isRefreshing ? <Spinner size="tiny" /> : 'Refresh'}
              </Button>
            )}
            
            {isAuthenticated ? (
              <Button
                appearance="secondary"
                onClick={handleLogout}
                style={{
                  background: 'rgba(239, 68, 68, 0.1)',
                  border: '1px solid rgba(239, 68, 68, 0.3)',
                  color: '#f87171',
                }}
              >
                Sign Out
              </Button>
            ) : (
              <Button
                appearance="primary"
                icon={<PersonLockRegular />}
                onClick={startOAuthFlow}
                disabled={isLoading}
                style={{
                  background: 'linear-gradient(135deg, #8b5cf6, #ec4899)',
                  border: 'none',
                  color: 'white',
                }}
              >
                {isLoading ? (
                  <>
                    <Spinner size="tiny" style={{ marginRight: 8 }} />
                    Authenticating...
                  </>
                ) : (
                  'Sign In with WindowsForum'
                )}
              </Button>
            )}
          </div>
        </div>

        {error && (
          <Text className={styles.error}>
            <ErrorCircleFilled fontSize={16} style={{ marginRight: 8 }} />
            {error}
          </Text>
        )}
      </Card>

      {isLoading && (
        <Card style={{
          padding: 16,
          background: 'rgba(59, 130, 246, 0.1)',
          border: '1px solid rgba(59, 130, 246, 0.3)',
        }}>
          <Text size={300} style={{ color: '#60a5fa' }}>
            <i className="fas fa-info-circle" style={{ marginRight: 8 }}></i>
            A browser window has opened for authentication. Please sign in to WindowsForum and authorize the application.
          </Text>
        </Card>
      )}

      {!isAuthenticated && (
        <Card style={{
          padding: 16,
          background: 'rgba(251, 191, 36, 0.1)',
          border: '1px solid rgba(251, 191, 36, 0.3)',
        }}>
          <Text size={200} weight="semibold" style={{ color: '#fbbf24', display: 'block', marginBottom: 8 }}>
            Secure OAuth2 Authentication
          </Text>
          <Text size={200} style={{ color: '#fcd34d' }}>
            • Your password is never stored locally
            <br />• Authentication happens directly on WindowsForum
            <br />• You can revoke access anytime from your forum account
            <br />• Tokens are automatically refreshed when needed
          </Text>
        </Card>
      )}
    </div>
  );
};

export default OAuthLogin;