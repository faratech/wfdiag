import React from 'react'
import ReactDOM from 'react-dom/client'
import { FluentProvider } from '@fluentui/react-components'
import App from './App'
import { ErrorBoundary } from './components/ErrorBoundary'
import { wfDarkTheme } from './theme'
import './index.css'

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <ErrorBoundary>
      <FluentProvider theme={wfDarkTheme}>
        <App />
      </FluentProvider>
    </ErrorBoundary>
  </React.StrictMode>,
)