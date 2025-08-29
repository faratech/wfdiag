import React, { useState } from 'react';
import {
  Dialog,
  DialogTrigger,
  DialogSurface,
  DialogTitle,
  DialogBody,
  DialogActions,
  DialogContent,
  Button,
  Input,
  Field,
  Spinner,
  Text,
  makeStyles,
  tokens,
} from '@fluentui/react-components';
import { PersonLockRegular, KeyRegular } from '@fluentui/react-icons';
import { invoke } from '@tauri-apps/api/core';

const useStyles = makeStyles({
  dialogContent: {
    display: 'flex',
    flexDirection: 'column',
    gap: tokens.spacingVerticalL,
  },
  field: {
    display: 'flex',
    flexDirection: 'column',
    gap: tokens.spacingVerticalXS,
  },
  error: {
    color: tokens.colorPaletteRedForeground1,
    fontSize: tokens.fontSizeBase200,
    marginTop: tokens.spacingVerticalXS,
  },
  success: {
    color: tokens.colorPaletteGreenForeground1,
    fontSize: tokens.fontSizeBase200,
  },
  info: {
    marginBottom: tokens.spacingVerticalM,
    padding: tokens.spacingVerticalS,
    backgroundColor: tokens.colorNeutralBackground2,
    borderRadius: tokens.borderRadiusMedium,
  },
});

interface LoginDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onLoginSuccess: (userInfo: any) => void;
}

export const LoginDialog: React.FC<LoginDialogProps> = ({
  open,
  onOpenChange,
  onLoginSuccess,
}) => {
  const styles = useStyles();
  const [username, setUsername] = useState('');
  const [password, setPassword] = useState('');
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleLogin = async () => {
    if (!username || !password) {
      setError('Please enter both username and password');
      return;
    }

    setLoading(true);
    setError(null);

    try {
      const response = await invoke('authenticate', {
        username,
        password,
      });

      if (response && (response as any).success) {
        onLoginSuccess(response);
        onOpenChange(false);
        setUsername('');
        setPassword('');
      } else {
        setError((response as any).error || 'Authentication failed');
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Login failed');
    } finally {
      setLoading(false);
    }
  };

  const handleKeyPress = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter' && !loading) {
      handleLogin();
    }
  };

  return (
    <Dialog open={open} onOpenChange={(_, data) => onOpenChange(data.open)}>
      <DialogSurface>
        <DialogBody>
          <DialogTitle>WindowsForum Login</DialogTitle>
          <DialogContent className={styles.dialogContent}>
            <div className={styles.info}>
              <Text size={200}>
                Sign in with your WindowsForum account to enable AI-powered system analysis.
                Your credentials are sent directly to WindowsForum for authentication.
              </Text>
            </div>

            <Field
              className={styles.field}
              label="Username"
              required
              validationState={error ? 'error' : undefined}
            >
              <Input
                contentBefore={<PersonLockRegular />}
                value={username}
                onChange={(_, data) => setUsername(data.value)}
                onKeyPress={handleKeyPress}
                disabled={loading}
                placeholder="Your WindowsForum username"
              />
            </Field>

            <Field
              className={styles.field}
              label="Password"
              required
              validationState={error ? 'error' : undefined}
            >
              <Input
                contentBefore={<KeyRegular />}
                type="password"
                value={password}
                onChange={(_, data) => setPassword(data.value)}
                onKeyPress={handleKeyPress}
                disabled={loading}
                placeholder="Your WindowsForum password"
              />
            </Field>

            {error && (
              <Text className={styles.error}>{error}</Text>
            )}

            <Text size={100} italic>
              Don't have an account?{' '}
              <a
                href="https://windowsforum.com/register"
                target="_blank"
                rel="noopener noreferrer"
              >
                Register at WindowsForum.com
              </a>
            </Text>
          </DialogContent>
          <DialogActions>
            <DialogTrigger disableButtonEnhancement>
              <Button appearance="secondary" disabled={loading}>
                Cancel
              </Button>
            </DialogTrigger>
            <Button
              appearance="primary"
              onClick={handleLogin}
              disabled={loading || !username || !password}
              icon={loading ? <Spinner size="tiny" /> : undefined}
            >
              {loading ? 'Signing in...' : 'Sign In'}
            </Button>
          </DialogActions>
        </DialogBody>
      </DialogSurface>
    </Dialog>
  );
};

export default LoginDialog;